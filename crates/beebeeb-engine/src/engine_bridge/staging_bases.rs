//! `FinderStagingBases`: where a Finder write's plaintext copy can be staged, split out of `landing.rs` (the
//! file-size ratchet). The folder under each base is the app identity's.

use std::path::PathBuf;

/// The folders a Finder write's plaintext copy can be staged in, from the bases this process
/// resolves: the app's data dir, its cache dir and the temp dir. Paths only: nothing is created
/// or touched until a root is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderStagingBases {
    /// The app's own folder name under each base (`EngineIdentity::config_dir_name`).
    pub folder: &'static str,
    pub data: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub temp: PathBuf,
}

impl FinderStagingBases {
    /// This process's bases: the OS's data, cache and temp dirs.
    #[cfg(not(any(test, feature = "test-support")))]
    pub fn current(identity: &crate::identity::EngineIdentity) -> Self {
        Self {
            folder: identity.config_dir_name(),
            data: dirs::data_dir(),
            cache: dirs::cache_dir(),
            temp: std::env::temp_dir(),
        }
    }

    /// A test build stages under a per-process sandbox, never the person's real data or cache
    /// dir: the installed app stages there too, and every test that queues a Finder write
    /// (engine bridge, IPC socket, watcher) would otherwise add plaintext files to it. A test
    /// points one bridge at bases of its own with `Seams::stage_under`. Pinned by
    /// `unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn current(identity: &crate::identity::EngineIdentity) -> Self {
        Self {
            folder: identity.config_dir_name(),
            data: Some(crate::test_sandbox::dir("data").expect("create the unit-test sandbox")),
            cache: Some(crate::test_sandbox::dir("cache").expect("create the unit-test sandbox")),
            temp: std::env::temp_dir(),
        }
    }

    /// `<data dir>/beebeeb/finder-writes`, where macOS stages: a folder the system does not
    /// purge. `None` when the OS gives no data dir.
    #[cfg_attr(not(any(target_os = "macos", test, feature = "test-support")), allow(dead_code))]
    pub fn durable_root(&self) -> Option<PathBuf> {
        self.data
            .as_ref()
            .map(|data| data.join(self.folder).join("finder-writes"))
    }

    /// `<cache dir>/beebeeb/finder-writes` (the temp dir when the OS gives no cache dir): where
    /// every platform staged before, and where Windows and Linux still stage.
    pub fn cache_root(&self) -> PathBuf {
        self.cache
            .as_ref()
            .unwrap_or(&self.temp)
            .join(self.folder)
            .join("finder-writes")
    }

    /// `<temp dir>/beebeeb/finder-writes`: the fallback when the cache root is not writable
    /// (Windows and Linux; macOS used it before as well).
    pub fn temp_root(&self) -> PathBuf {
        self.temp.join(self.folder).join("finder-writes")
    }

    /// Every folder a staged copy can be in. On macOS: the data root, then the cache root and the
    /// temp root, which builds before the move staged into, so their copies are still found. Elsewhere:
    /// the cache root and the temp root.
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::with_capacity(3);
        #[cfg(target_os = "macos")]
        candidates.extend(self.durable_root());
        candidates.extend([self.cache_root(), self.temp_root()]);
        candidates
    }
}
