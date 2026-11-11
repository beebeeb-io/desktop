//! The real clock, the updater's record on disk, and the installer launch. Small on purpose: every rule lives in
//! `policy` and `machine`, which are tested without these.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::machine::{Clock, Persisted, Store};

/// When this build was made, unix seconds: `BEEBEEB_BUILD_NOT_BEFORE` in the environment of the release build
/// (`release-native-windows.yml` sets it to the commit's time before `cargo build`; `build.rs` stops the build on a
/// value that is set and is not a plausible time). `None` in any other build.
pub const BUILD_NOT_BEFORE: Option<u64> = parse_build_time(option_env!("BEEBEEB_BUILD_NOT_BEFORE"));

/// `beebeeb-updater-not-before=<unix seconds>;` (or `…=unset;`), written by `build.rs`. The release workflow reads
/// it out of the built binary and refuses to publish unless it is the commit's time: what is checked is the
/// binary that ships, not the environment it was meant to be built in.
pub static BUILD_MARK: &str = env!("BEEBEEB_UPDATER_BUILD_MARK");

/// Plain digits, at least [`crate::policy::V2_KEY_CREATED`]; anything else is "not a release build".
pub(crate) const fn parse_build_time(text: Option<&str>) -> Option<u64> {
    let Some(text) = text else { return None };
    let digits = text.as_bytes();
    if digits.is_empty() || digits.len() > 12 {
        return None;
    }
    let (mut value, mut i) = (0u64, 0);
    while i < digits.len() {
        if !digits[i].is_ascii_digit() {
            return None;
        }
        value = value * 10 + (digits[i] - b'0') as u64;
        i += 1;
    }
    if value < crate::policy::V2_KEY_CREATED {
        return None;
    }
    Some(value)
}

/// Why no trust could be built at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotAReleaseBuild {
    /// `keys/updater-v2.pub` is not a minisign public key.
    NoUpdaterKey,
}

/// The trust a released native Windows build ships with: the v2 key only, for 0.9.0 and later; our host; the NSIS
/// target first; nothing signed before the build itself.
///
/// A build without a build time (a development build) still gets a trust, with `not_before` 0, and with it every
/// check ends in `Failed { Refused(NoBuildTime) }`: the shell shows that state. It fails closed and in sight; the
/// rule is never off.
pub fn windows_release_trust() -> Result<crate::policy::Trust, NotAReleaseBuild> {
    // Keeps the mark in every binary that can update itself (the release workflow looks for it there).
    std::hint::black_box(BUILD_MARK);
    windows_trust_built_at(BUILD_NOT_BEFORE.unwrap_or(0))
}

/// The same trust with a given not-before. The bridge release 0.8.25 uses this to check a v2 manifest before it
/// installs a native build, with the fixed [`crate::policy::V2_KEY_CREATED`] (it bakes in no time of its own).
pub fn windows_trust_built_at(not_before: u64) -> Result<crate::policy::Trust, NotAReleaseBuild> {
    let key = crate::signature::PublicKey::updater_v2().map_err(|_| NotAReleaseBuild::NoUpdaterKey)?;
    Ok(crate::policy::Trust {
        keys: vec![crate::policy::TrustedKey {
            key,
            valid_for: crate::policy::KeyScope::from(crate::policy::NATIVE_FLOOR),
        }],
        product: "desktop".to_string(),
        allowed_hosts: vec![RELEASE_HOST.to_string()],
        targets: vec!["windows-x86_64-nsis".to_string(), "windows-x86_64".to_string()],
        floor: crate::policy::NATIVE_FLOOR,
        not_before,
    })
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

/// Opens the staged installer for reading so that, while the handle lives, no other program can write, replace,
/// rename or delete the file (Windows: share mode read only, so no `FILE_SHARE_WRITE` and no `FILE_SHARE_DELETE`).
/// The updater verifies the file through this handle and keeps it open until the installer process exists, so the
/// bytes that were verified are the bytes that run. Elsewhere it is a plain open: no installer is started there.
pub fn open_denying_writes(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Channel;
    use crate::policy::HighWater;
    use crate::version::Version;

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
    fn a_release_build_trusts_one_key_one_host_and_never_goes_below_0_9_0() {
        assert!(RELEASE_BASE_URL.ends_with(RELEASE_HOST));
        // The tests are not a release build: no build time is baked in, and the trust that comes out of that
        // refuses every manifest by name (policy_tests: `a_build_without_a_build_time_takes_no_update`).
        assert_eq!(BUILD_NOT_BEFORE, None);
        assert_eq!(BUILD_MARK, "beebeeb-updater-not-before=unset;");
        assert_eq!(windows_release_trust().expect("the v2 key is committed").not_before, 0);
        assert!(include_str!("../build.rs").contains("const EARLIEST: u64 = 1_794_355_200;"));
        assert_eq!(crate::policy::V2_KEY_CREATED, 1_794_355_200);
        let trust = windows_trust_built_at(1_800_000_000).expect("the v2 key is committed");
        assert_eq!(trust.keys.len(), 1, "the native line trusts the v2 key only");
        assert!(trust.keys[0].valid_for.covers(crate::policy::NATIVE_FLOOR));
        assert!(!trust.keys[0].valid_for.covers(Version::new(0, 8, 25)));
        assert!(trust.signer(&legacy_signature()).is_none(), "not the 0.8.x key");
        assert_eq!(trust.allowed_hosts, ["releases.beebeeb.io"]);
        assert_eq!(trust.floor.to_string(), "0.9.0");
        assert_eq!(trust.targets, ["windows-x86_64-nsis", "windows-x86_64"]);
        assert_eq!(trust.not_before, 1_800_000_000);
    }

    /// A signature the 0.8.x key really made (the published 0.8.22 stable manifest).
    fn legacy_signature() -> crate::signature::Signature {
        let manifest: serde_json::Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/desktop-latest-0.8.22.json")).expect("json");
        let signature = manifest["platforms"]["windows-x86_64-nsis"]["signature"]
            .as_str()
            .expect("a signature");
        crate::signature::Signature::from_base64(signature).expect("decodes")
    }

    #[test]
    fn the_build_time_is_plain_digits_or_nothing() {
        assert_eq!(parse_build_time(Some("1800000000")), Some(1_800_000_000));
        assert_eq!(
            parse_build_time(Some("1794355200")),
            Some(crate::policy::V2_KEY_CREATED)
        );
        for bad in [
            None,
            Some(""),
            // Before the day the v2 key was made: digits, but not a build time.
            Some("0"),
            Some("1794355199"),
            Some("18e8"),
            Some(" 1800000000"),
            Some("-1"),
            Some("1800000000000"),
        ] {
            assert_eq!(parse_build_time(bad), None, "{bad:?}");
        }
    }

    /// Windows only (the lead's native run): while the updater holds the staged installer, no other program can
    /// write, replace, rename or delete it, and it can still be read (which is what starting it needs).
    #[cfg(windows)]
    #[test]
    fn a_held_installer_cannot_be_written_replaced_renamed_or_deleted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("Beebeeb_0.9.1_x64-setup.exe");
        fs::write(&path, b"verified bytes").expect("write");
        let held = open_denying_writes(&path).expect("open");
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err(), "write");
        assert!(fs::write(&path, b"other bytes").is_err(), "replace");
        assert!(fs::rename(&path, dir.path().join("moved.exe")).is_err(), "rename away");
        fs::write(dir.path().join("other.exe"), b"other bytes").expect("write the other file");
        assert!(fs::rename(dir.path().join("other.exe"), &path).is_err(), "rename over");
        assert!(fs::remove_file(&path).is_err(), "delete");
        assert_eq!(fs::read(&path).expect("still readable"), b"verified bytes");
        drop(held);
        fs::write(&path, b"other bytes").expect("writable again once released");
    }
}
