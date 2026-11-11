//! This device as the backup's identity rule sees it (spec §4.2 rules 2 and 3): its name, its id and its platform,
//! for core's `find_device_folder`.
//!
//! **The name is today's name source** (`runner::device_name`, the `hostname` crate: on Windows
//! `GetComputerNameExW(ComputerNamePhysicalDnsHostname)`, the case the person gave the PC), never
//! `GetComputerNameW`, which answers the upper-case NetBIOS name. The old mirror named `Backups/<device>` from this
//! source through `known_folder::sanitize_device_segment`, and core adopts a manifest-less folder only under the
//! exact name (`DeviceFolderChoice::AdoptLegacy`): a name from another source would start a second device folder
//! next to `Backups/GLegion` (lead finding, 2026-11-06).

use beebeeb_core::backup::DeviceSelf;
use beebeeb_types::backup::Platform;

/// This platform, as the identity rule names it.
pub const PLATFORM: Platform = if cfg!(target_os = "windows") {
    Platform::Windows
} else if cfg!(target_os = "macos") {
    Platform::Macos
} else {
    Platform::Linux
};

/// The OS's stable machine id: Windows `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`, Linux
/// `/etc/machine-id` (else `/var/lib/dbus/machine-id`). `None` when there is none: then this device has no
/// backup id and no pass runs. macOS (`IOPlatformUUID`) comes with Task D5.
pub fn machine_id() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows_machine_guid()
    }
    #[cfg(target_os = "linux")]
    {
        ["/etc/machine-id", "/var/lib/dbus/machine-id"]
            .iter()
            .find_map(|path| machine_id_from_file(std::path::Path::new(path)))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// A machine-id file's id: its first line, trimmed; `None` when it is missing, unreadable or empty.
#[cfg_attr(not(any(any(test, feature = "test-support"), target_os = "linux")), allow(dead_code))]
fn machine_id_from_file(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.lines().next()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(target_os = "windows")]
fn windows_machine_guid() -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW};
    use windows::core::w;
    let mut buf = [0u16; 64];
    let mut bytes = u32::try_from(std::mem::size_of_val(&buf)).ok()?;
    // SAFETY: `buf` is a live, writable buffer of `bytes` bytes; the value is read as REG_SZ from the 64-bit view.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Cryptography"),
            w!("MachineGuid"),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = usize::try_from(bytes).ok()? / 2;
    let text = String::from_utf16(&buf[..units.min(buf.len())]).ok()?;
    let id = text.trim_end_matches('\0').trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// The device's name for the identity rule: today's source, the one the old mirror named `Backups/<device>` from
/// (the mirror, removed in LD-0113, called `runner::device_name` too).
fn backup_device_name() -> Option<String> {
    crate::runner::device_name()
}

/// This device for `find_device_folder`, or `None` when the OS gives no usable name or machine id (then no device
/// folder is chosen and no pass runs, as the old mirror made nothing for a lossy name).
pub fn device_self() -> Option<DeviceSelf> {
    device_self_from(backup_device_name()?, &machine_id()?)
}

fn device_self_from(name: String, machine_id: &str) -> Option<DeviceSelf> {
    Some(DeviceSelf {
        device_id: beebeeb_core::backup::device_id_from_machine_id(machine_id),
        name,
        platform: PLATFORM,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_core::backup::{DeviceFolderCandidate, DeviceFolderChoice, FolderManifest, find_device_folder};

    fn legacy_folder(name: &str) -> DeviceFolderCandidate {
        DeviceFolderCandidate {
            folder_id: uuid::Uuid::parse_str("0193b1a6-0000-7000-8000-00000000a001").unwrap(),
            // Exactly what the old mirror named it (`<sync_root>/Backups/<sanitize_device_segment(name)>`).
            name: crate::known_folder::sanitize_device_segment(name),
            manifest: FolderManifest::Absent,
        }
    }

    /// The old mirror's `Backups/GLegion` is adopted, never doubled, when the device's name comes from today's
    /// source. The upper-case NetBIOS name (`GetComputerNameW`) would start `GLEGION (…)` next to it: the reason the
    /// source is fixed here.
    #[test]
    fn legacy_mirror_folder_is_adopted_with_todays_name_source() {
        let folder = legacy_folder("GLegion");
        let windows = |name: &str| DeviceSelf {
            device_id: beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0042-3510-8052-b4c04f4e3732"),
            name: name.to_string(),
            platform: Platform::Windows,
        };
        assert_eq!(
            find_device_folder(&windows("GLegion"), std::slice::from_ref(&folder)),
            DeviceFolderChoice::AdoptLegacy(folder.folder_id)
        );
        match find_device_folder(&windows("GLEGION"), std::slice::from_ref(&folder)) {
            DeviceFolderChoice::Create { name } => assert!(name.starts_with("GLEGION ("), "{name}"),
            other => panic!("the NetBIOS name is expected to double the folder, got {other:?}"),
        }

        // This machine: the folder the old mirror made here (named from `runner::device_name`, as the mirror does)
        // is adopted by the name `device_self` uses.
        let mine = legacy_folder(&crate::runner::device_name().expect("the test machine has a name"));
        let me = device_self_from(backup_device_name().unwrap(), "test-machine").unwrap();
        let me_on_windows = DeviceSelf {
            platform: Platform::Windows,
            ..me
        };
        assert_eq!(
            find_device_folder(&me_on_windows, std::slice::from_ref(&mine)),
            DeviceFolderChoice::AdoptLegacy(mine.folder_id),
            "the name source and the old mirror's folder disagree"
        );
    }

    /// Every name a Windows computer can have (letters, digits and hyphens, at most 15 characters for the NetBIOS
    /// form, up to 63 for the DNS one) gets the same folder name from the old sanitizer and from core's, so every
    /// old mirror folder is adopted. Names outside that alphabet differ (PR notes, "Legacy adoption").
    #[test]
    fn old_and_new_sanitizers_agree_on_windows_computer_names() {
        let me = |name: &str| DeviceSelf {
            device_id: beebeeb_core::backup::device_id_from_machine_id("machine"),
            name: name.to_string(),
            platform: Platform::Windows,
        };
        for name in [
            "GLegion",
            "DESKTOP-4F2K9QZ",
            "laptop-van-guus",
            "PC1",
            "a",
            "x-1-y-2-z-3-w-4-v-5-u-6-t-7-s-8-r",
        ] {
            let folder = legacy_folder(name);
            assert_eq!(
                find_device_folder(&me(name), std::slice::from_ref(&folder)),
                DeviceFolderChoice::AdoptLegacy(folder.folder_id),
                "{name}"
            );
        }
    }

    /// Native rung: the registry read gives this PC's `MachineGuid`, a UUID, and the id derived from it is stable.
    #[cfg(target_os = "windows")]
    #[test]
    fn machine_guid_is_read_on_windows() {
        let id = machine_id().expect("HKLM\\SOFTWARE\\Microsoft\\Cryptography\\MachineGuid is readable");
        assert!(
            uuid::Uuid::parse_str(&id).is_ok(),
            "MachineGuid is a UUID (got {} chars)",
            id.len()
        );
        assert_eq!(machine_id().as_deref(), Some(id.as_str()), "the same id every time");
        let me = device_self().expect("a name and a machine id");
        assert_eq!(me.device_id, beebeeb_core::backup::device_id_from_machine_id(&id));
        assert_eq!(me.platform, Platform::Windows);
    }

    #[test]
    fn machine_id_reads_the_first_line_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine-id");
        std::fs::write(&path, "  0123456789abcdef0123456789abcdef \n").unwrap();
        assert_eq!(
            machine_id_from_file(&path).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        std::fs::write(&path, "\n").unwrap();
        assert_eq!(machine_id_from_file(&path), None, "an empty id is no id");
        assert_eq!(machine_id_from_file(&dir.path().join("missing")), None);
    }
}
