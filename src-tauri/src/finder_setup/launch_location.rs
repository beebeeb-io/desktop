//! Where the app runs from (spec §6.2 "The launch-location check"). Pure: the bundle path and
//! the REAL home are passed in. Under the app sandbox `$HOME` is the app container, so the
//! caller passes `ipc_socket::macos_real_home_dir()`, never `dirs::home_dir()`.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchLocation {
    /// An Applications folder: `/Applications/…`, `~/Applications/…`, or the Applications
    /// folder of another disk (`/Volumes/<name>/Applications/…`). A home that itself lives on
    /// an external disk counts, because `~` is the real home passed in.
    Applications,
    /// Gatekeeper App Translocation (a quarantined app opened where it was downloaded).
    Translocated,
    /// A bundle sitting directly at a volume's root (`/Volumes/<name>/Beebeeb.app`): the
    /// mounted disk image. Nothing deeper under `/Volumes` counts (lead ruling 2026-10-06).
    DiskImage,
    /// Anywhere else: proceed and let macOS decide (a -2002 is still classified).
    Elsewhere,
}

impl LaunchLocation {
    /// Whether `addDomain` may be called from here.
    pub fn allows_add(self) -> bool {
        !matches!(self, Self::Translocated | Self::DiskImage)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applications => "applications",
            Self::Translocated => "translocated",
            Self::DiskImage => "disk_image",
            Self::Elsewhere => "elsewhere",
        }
    }
}

pub fn launch_location(bundle: &Path, real_home: &Path) -> LaunchLocation {
    if bundle
        .components()
        .any(|c| c == Component::Normal("AppTranslocation".as_ref()))
    {
        return LaunchLocation::Translocated;
    }
    if is_at_volume_root(bundle) {
        return LaunchLocation::DiskImage;
    }
    if bundle.starts_with("/Applications")
        || bundle.starts_with(real_home.join("Applications"))
        || is_under_volume_applications(bundle)
    {
        return LaunchLocation::Applications;
    }
    LaunchLocation::Elsewhere
}

/// `/Volumes/<name>/Beebeeb.app`: the bundle's parent directory is exactly one component
/// below `/Volumes`, which is how a mounted disk image presents its app.
fn is_at_volume_root(bundle: &Path) -> bool {
    bundle.parent().and_then(Path::parent) == Some(Path::new("/Volumes"))
}

/// `/Volumes/<name>/Applications/…`: the Applications folder of another disk.
fn is_under_volume_applications(bundle: &Path) -> bool {
    let Ok(on_disk) = bundle.strip_prefix("/Volumes") else {
        return false;
    };
    let mut parts = on_disk.components();
    parts.next(); // <name>
    matches!(parts.next(), Some(Component::Normal(folder)) if folder == "Applications")
}

/// `…/Beebeeb.app/Contents/MacOS/beebeeb` → `…/Beebeeb.app`; `None` for a bare dev binary
/// (`target/debug/beebeeb`), which therefore counts as `Elsewhere`.
pub fn bundle_path_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// The running app's bundle, read when needed ("Show in Finder").
#[cfg(target_os = "macos")]
pub fn current_bundle_path() -> Option<PathBuf> {
    std::env::current_exe().ok().as_deref().and_then(bundle_path_from_exe)
}

/// The running app's location, read once at launch.
#[cfg(target_os = "macos")]
pub fn current() -> LaunchLocation {
    match current_bundle_path() {
        Some(bundle) => launch_location(&bundle, &crate::ipc_socket::macos_real_home_dir()),
        None => LaunchLocation::Elsewhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/sam";

    fn at(path: &str) -> LaunchLocation {
        launch_location(Path::new(path), Path::new(HOME))
    }

    #[test]
    fn the_five_paths_of_section_13() {
        assert_eq!(at("/Applications/Beebeeb.app"), LaunchLocation::Applications);
        assert_eq!(at("/Users/sam/Applications/Beebeeb.app"), LaunchLocation::Applications);
        assert_eq!(
            at("/private/var/folders/x7/abc123/T/AppTranslocation/6F0E1C2D-0000-4000-8000-000000000000/d/Beebeeb.app"),
            LaunchLocation::Translocated
        );
        assert_eq!(at("/Volumes/Beebeeb/Beebeeb.app"), LaunchLocation::DiskImage);
        assert_eq!(at("/Users/sam/Downloads/Beebeeb.app"), LaunchLocation::Elsewhere);
    }

    #[test]
    fn only_translocated_and_disk_image_block_add() {
        assert!(LaunchLocation::Applications.allows_add());
        assert!(LaunchLocation::Elsewhere.allows_add());
        assert!(!LaunchLocation::Translocated.allows_add());
        assert!(!LaunchLocation::DiskImage.allows_add());
    }

    #[test]
    fn matching_is_by_path_component_not_by_text() {
        assert_eq!(at("/ApplicationsOld/Beebeeb.app"), LaunchLocation::Elsewhere);
        assert_eq!(
            at("/Users/sam/Applications Backup/Beebeeb.app"),
            LaunchLocation::Elsewhere
        );
        assert_eq!(at("/Users/samuel/Applications/Beebeeb.app"), LaunchLocation::Elsewhere);
        assert_eq!(at("/Applications/Utilities/Beebeeb.app"), LaunchLocation::Applications);
    }

    #[test]
    fn home_applications_needs_the_real_home_not_the_sandbox_container() {
        assert_eq!(
            at("/Users/sam/Library/Containers/io.beebeeb.app/Data/Applications/Beebeeb.app"),
            LaunchLocation::Elsewhere
        );
    }

    /// Lead ruling 2026-10-06, replacing spec §6.2's "starting with `/Volumes/`" (plan "Spec
    /// issues" 11): only a bundle sitting DIRECTLY at a volume's root is the mounted disk image.
    #[test]
    fn a_bundle_at_a_volume_root_is_the_mounted_disk_image() {
        assert_eq!(at("/Volumes/Beebeeb/Beebeeb.app"), LaunchLocation::DiskImage);
        assert_eq!(at("/Volumes/Beebeeb 1/Beebeeb.app"), LaunchLocation::DiskImage);
    }

    #[test]
    fn an_applications_folder_on_another_disk_is_a_real_install() {
        assert_eq!(
            at("/Volumes/Data/Applications/Beebeeb.app"),
            LaunchLocation::Applications
        );
    }

    #[test]
    fn a_home_on_an_external_disk_is_a_real_install() {
        let home = Path::new("/Volumes/Ext/sam");
        assert_eq!(
            launch_location(Path::new("/Volumes/Ext/sam/Applications/Beebeeb.app"), home),
            LaunchLocation::Applications
        );
    }

    #[test]
    fn any_other_place_on_a_disk_is_left_to_macos() {
        assert_eq!(at("/Volumes/Ext/Tools/Beebeeb.app"), LaunchLocation::Elsewhere);
    }

    #[test]
    fn bundle_path_from_exe_finds_the_app_bundle() {
        assert_eq!(
            bundle_path_from_exe(Path::new("/Applications/Beebeeb.app/Contents/MacOS/beebeeb")),
            Some(PathBuf::from("/Applications/Beebeeb.app"))
        );
        assert_eq!(
            bundle_path_from_exe(Path::new("/Users/sam/src/desktop/src-tauri/target/debug/beebeeb")),
            None
        );
    }

    #[test]
    fn names_match_what_serde_writes() {
        for location in [
            LaunchLocation::Applications,
            LaunchLocation::Translocated,
            LaunchLocation::DiskImage,
            LaunchLocation::Elsewhere,
        ] {
            assert_eq!(
                serde_json::to_value(location).unwrap(),
                serde_json::Value::String(location.as_str().into())
            );
        }
    }
}
