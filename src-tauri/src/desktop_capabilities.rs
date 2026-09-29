//! Host support, deliberately independent of WebView layout/query parameters.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOs { Macos, Windows, Linux, Unknown }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemIntegration { FileProvider, CloudFiles, None }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability { Available, Unavailable, Unknown }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SharingLevel { RecipientReadOnly, WebOnly }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallFormat { App, Nsis, Msi, Appimage, Deb, Rpm, Unknown }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFormat { AppBundle, Nsis, Msi, Appimage, PackageManager, Manual }

#[derive(Debug, Serialize)]
pub struct DesktopCapabilities {
    pub host_os: HostOs,
    pub filesystem_integration: FilesystemIntegration,
    pub on_demand: bool,
    /// Implemented native backend, not a claim the user's vault is unlocked.
    pub native_credentials: bool,
    pub tray: Availability,
    pub sharing: SharingLevel,
    pub install_format: InstallFormat,
    pub update_format: UpdateFormat,
}

pub fn host_os() -> HostOs {
    if cfg!(target_os = "macos") { HostOs::Macos }
    else if cfg!(target_os = "windows") { HostOs::Windows }
    else if cfg!(target_os = "linux") { HostOs::Linux }
    else { HostOs::Unknown }
}

pub fn snapshot(host: HostOs, install_format: InstallFormat, tray_created: bool) -> DesktopCapabilities {
    let filesystem_integration = match host {
        HostOs::Macos => FilesystemIntegration::FileProvider,
        HostOs::Windows => FilesystemIntegration::CloudFiles,
        // The Linux FUSE prototype is not mounted by runner. Do not advertise it.
        _ => FilesystemIntegration::None,
    };
    let native_credentials = matches!(host, HostOs::Macos | HostOs::Windows);
    let update_format = match (host, install_format) {
        (HostOs::Macos, InstallFormat::App) => UpdateFormat::AppBundle,
        (HostOs::Windows, InstallFormat::Nsis) => UpdateFormat::Nsis,
        (HostOs::Windows, InstallFormat::Msi) => UpdateFormat::Msi,
        (HostOs::Linux, InstallFormat::Appimage) => UpdateFormat::Appimage,
        (HostOs::Linux, InstallFormat::Deb | InstallFormat::Rpm) => UpdateFormat::PackageManager,
        _ => UpdateFormat::Manual,
    };
    DesktopCapabilities {
        host_os: host, filesystem_integration,
        on_demand: filesystem_integration != FilesystemIntegration::None,
        native_credentials,
        tray: if !tray_created { Availability::Unavailable }
              else if matches!(host, HostOs::Macos | HostOs::Windows) { Availability::Available }
              else { Availability::Unknown }, // Indicator object does not prove Linux panel support.
        sharing: if native_credentials { SharingLevel::RecipientReadOnly } else { SharingLevel::WebOnly },
        install_format, update_format,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_wire_fixtures_cover_hosts_and_packages() {
        let cases = [
            ("macos", HostOs::Macos, InstallFormat::App),
            ("windows-nsis", HostOs::Windows, InstallFormat::Nsis),
            ("windows-msi", HostOs::Windows, InstallFormat::Msi),
            ("linux-appimage", HostOs::Linux, InstallFormat::Appimage),
            ("linux-deb", HostOs::Linux, InstallFormat::Deb),
            ("linux-rpm", HostOs::Linux, InstallFormat::Rpm),
            ("unknown", HostOs::Unknown, InstallFormat::Unknown),
        ];
        let fixtures: serde_json::Value = serde_json::from_str(include_str!("../../tests/fixtures/desktop-capabilities.json")).unwrap();
        assert_eq!(fixtures.as_object().unwrap().len(), cases.len());
        for (name, host, format) in cases {
            assert_eq!(serde_json::to_value(snapshot(host, format, true)).unwrap(), fixtures[name], "{name}");
        }
    }

    #[test]
    fn capability_unknown_or_mismatched_packages_never_enable_updater() {
        for host in [HostOs::Macos, HostOs::Windows, HostOs::Linux, HostOs::Unknown] {
            assert_eq!(snapshot(host, InstallFormat::Unknown, false).update_format, UpdateFormat::Manual);
            assert_eq!(snapshot(host, InstallFormat::Unknown, false).tray, Availability::Unavailable);
        }
        assert_eq!(snapshot(HostOs::Linux, InstallFormat::Msi, true).update_format, UpdateFormat::Manual);
    }

    #[test]
    fn capability_host_is_the_compilation_target() {
        let expected = match std::env::consts::OS {
            "macos" => HostOs::Macos, "windows" => HostOs::Windows,
            "linux" => HostOs::Linux, _ => HostOs::Unknown,
        };
        assert_eq!(host_os(), expected);
    }
}
