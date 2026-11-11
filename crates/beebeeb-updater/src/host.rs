//! The real clock, the updater's record on disk, and the installer launch. Small on purpose: every rule lives in
//! `policy` and `machine`, which are tested without these.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::machine::{Clock, Persisted, Store};

/// The trust a released Windows build ships with: our key, our host, the NSIS target first.
pub fn windows_release_trust() -> crate::policy::Trust {
    crate::policy::Trust {
        key: crate::signature::PublicKey::beebeeb_desktop(),
        product: "desktop".to_string(),
        allowed_hosts: vec![RELEASE_HOST.to_string()],
        targets: vec!["windows-x86_64-nsis".to_string(), "windows-x86_64".to_string()],
        floor: crate::policy::NATIVE_FLOOR,
    }
}

/// The one host a manifest or artefact may come from.
pub const RELEASE_HOST: &str = "releases.beebeeb.io";
pub const RELEASE_BASE_URL: &str = "https://releases.beebeeb.io";

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0)
    }
}

/// [`Persisted`] as JSON in one file (`%LOCALAPPDATA%\io.beebeeb.app\updater.json`), replaced atomically.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Store for FileStore {
    /// A missing or unreadable record reads as empty: the replay marks start again from the next accepted manifest.
    /// Refusing instead would leave an install that can never update again after one bad write.
    fn load(&self) -> Persisted {
        fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&mut self, persisted: &Persisted) -> bool {
        write_atomically(&self.path, persisted).is_ok()
    }
}

fn write_atomically(path: &Path, persisted: &Persisted) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temp)?;
    file.write_all(&serde_json::to_vec(persisted)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)
}

/// Starts the staged NSIS installer the way Tauri's updater does for 0.8.x (`/P` passive: progress only; `/R`
/// start the app afterwards; `/UPDATE` keep shortcuts and the autostart entry). A per-user install: no elevation.
#[cfg(windows)]
pub struct NsisInstaller;

#[cfg(windows)]
impl crate::machine::Apply for NsisInstaller {
    fn launch(&mut self, installer: &Path, _offer: &crate::policy::Offer) -> bool {
        std::process::Command::new(installer)
            .args(["/P", "/R", "/UPDATE"])
            .spawn()
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Channel;
    use crate::policy::HighWater;

    #[test]
    fn the_record_survives_a_restart_and_a_broken_file_reads_as_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("updater.json");
        let mut store = FileStore::new(&path);
        assert_eq!(store.load(), Persisted::default());

        let mut persisted = Persisted::default();
        persisted.seen.insert(
            Channel::Stable,
            HighWater {
                manifest_time: 1_800_000_000,
            },
        );
        persisted.last_check = Some(1_800_000_500);
        assert!(store.save(&persisted));
        assert_eq!(FileStore::new(&path).load(), persisted);
        assert_eq!(
            fs::read_dir(path.parent().expect("parent")).expect("listing").count(),
            1,
            "no temp file is left"
        );

        fs::write(&path, b"{ not json").expect("overwrite");
        assert_eq!(FileStore::new(&path).load(), Persisted::default());
    }

    #[test]
    fn a_release_build_trusts_one_host_and_never_goes_below_0_9_0() {
        let trust = windows_release_trust();
        assert_eq!(trust.allowed_hosts, ["releases.beebeeb.io"]);
        assert_eq!(trust.floor.to_string(), "0.9.0");
        assert_eq!(trust.targets, ["windows-x86_64-nsis", "windows-x86_64"]);
        assert!(RELEASE_BASE_URL.ends_with(RELEASE_HOST));
    }
}
