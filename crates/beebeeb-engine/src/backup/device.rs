//! This device as the backup's identity rule sees it (spec §4.2 rules 2 and 3): its name, its id and its platform,
//! for core's `find_device_folder`.
//!
//! **The name is today's name source** (`runner::device_name`, the `hostname` crate: on Windows
//! `GetComputerNameExW(ComputerNamePhysicalDnsHostname)`, the case the person gave the PC), never
//! `GetComputerNameW`, which answers the upper-case NetBIOS name. The old mirror named `Backups/<device>` from this
//! source through `known_folder::sanitize_device_segment`, and core adopts a manifest-less folder only under the
//! exact name (`DeviceFolderChoice::AdoptLegacy`): a name from another source would start a second device folder
//! next to `Backups/TestPC` (lead finding, 2026-11-06).

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
/// backup id and no pass runs. macOS: `IOPlatformUUID` from IOKit (no `ioreg` process; works in the App Sandbox).
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
    #[cfg(target_os = "macos")]
    {
        mac::platform_uuid()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// macOS: the IOKit / SystemConfiguration reads, by direct FFI (no extra crates).
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CString};

    type CFTypeRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFTypeRef;
        fn CFStringGetLength(s: CFTypeRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(len: isize, enc: u32) -> isize;
        fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: isize, enc: u32) -> u8;
        fn CFGetTypeID(c: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFRelease(c: CFTypeRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
        fn IORegistryEntryCreateCFProperty(entry: u32, key: CFTypeRef, alloc: CFTypeRef, options: u32) -> CFTypeRef;
        fn IOObjectRelease(obj: u32) -> i32;
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyComputerName(store: CFTypeRef, encoding: *mut u32) -> CFTypeRef;
    }

    /// An owned CF string, converted to a Rust `String` and released. `None` for null, a non-string or an empty one.
    unsafe fn take_string(cf: CFTypeRef) -> Option<String> {
        if cf.is_null() {
            return None;
        }
        // SAFETY: `cf` is a live, owned CF object; it is released exactly once, here.
        let out = unsafe {
            if CFGetTypeID(cf) == CFStringGetTypeID() {
                let cap = CFStringGetMaximumSizeForEncoding(CFStringGetLength(cf), UTF8) + 1;
                let mut buf = vec![0u8; usize::try_from(cap).unwrap_or(0).max(1)];
                if CFStringGetCString(cf, buf.as_mut_ptr().cast(), cap, UTF8) != 0 {
                    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                    String::from_utf8(buf[..end].to_vec()).ok()
                } else {
                    None
                }
            } else {
                None
            }
        };
        unsafe { CFRelease(cf) };
        out.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// `IOPlatformUUID` of the `IOPlatformExpertDevice`.
    pub fn platform_uuid() -> Option<String> {
        let class = CString::new("IOPlatformExpertDevice").ok()?;
        let key = CString::new("IOPlatformUUID").ok()?;
        // SAFETY: plain IOKit/CF calls; the matching dictionary is consumed by `IOServiceGetMatchingService`, the
        // service and the key and the property are each released once. Main port 0 is `kIOMainPortDefault`.
        unsafe {
            let service = IOServiceGetMatchingService(0, IOServiceMatching(class.as_ptr()));
            if service == 0 {
                return None;
            }
            let cf_key = CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), UTF8);
            let value = if cf_key.is_null() {
                std::ptr::null()
            } else {
                let v = IORegistryEntryCreateCFProperty(service, cf_key, std::ptr::null(), 0);
                CFRelease(cf_key);
                v
            };
            IOObjectRelease(service);
            take_string(value)
        }
    }

    /// The computer name ("Guus's MacBook Pro"), as System Settings > Sharing shows it.
    pub fn computer_name() -> Option<String> {
        // SAFETY: a null store asks for the current user's; the returned name is owned and released in `take_string`.
        unsafe { take_string(SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut())) }
    }
}

/// A machine-id file's id: its first line, trimmed; `None` when it is missing, unreadable or empty.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
fn machine_id_from_file(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.lines().next()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(target_os = "windows")]
fn windows_machine_guid() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY};
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
    // macOS: the computer name the person set; the host name (`Guuss-MacBook-Pro.local`) only when there is none.
    #[cfg(target_os = "macos")]
    if let Some(name) = mac::computer_name() {
        return Some(name);
    }
    crate::runner::device_name()
}

/// This device for `find_device_folder`, or `None` when the OS gives no usable name or machine id (then no device
/// folder is chosen and no pass runs, as the old mirror made nothing for a lossy name).
///
/// The test build is another device on the same machine: its id comes from the machine id under the identity's
/// salt and its name carries the identity's suffix (`TestPC (test)`), so it never finds the real app's device
/// folder by id and never adopts a folder the old mirror wrote under the bare name ([`may_adopt_legacy_folder`]).
pub fn device_self(identity: &crate::identity::EngineIdentity) -> Option<DeviceSelf> {
    device_self_from(identity, backup_device_name()?, &machine_id()?)
}

/// This device's name as [`device_self`] gives it (the identity's suffix included), also when no machine id gives it
/// a device id: for the rule that keeps a PC's own folder out of "Delete this backup" on such a PC.
pub fn device_name_for(identity: &crate::identity::EngineIdentity) -> Option<String> {
    Some(named(identity, backup_device_name()?))
}

/// `name` with the identity's suffix (`TestPC (test)`).
fn named(identity: &crate::identity::EngineIdentity, name: String) -> String {
    format!("{name}{}", identity.backup_device_name_suffix())
}

pub(crate) fn device_self_from(
    identity: &crate::identity::EngineIdentity,
    name: String,
    machine_id: &str,
) -> Option<DeviceSelf> {
    let salt = identity.backup_device_salt();
    let device_id = if salt.is_empty() {
        beebeeb_core::backup::device_id_from_machine_id(machine_id)
    } else {
        beebeeb_core::backup::device_id_from_machine_id(&format!("{salt}:{machine_id}"))
    };
    Some(DeviceSelf {
        device_id,
        name: named(identity, name),
        platform: PLATFORM,
    })
}

/// Only the real app adopts a manifest-less device folder (the old mirror's `Backups/<device>`): that folder is
/// the real app's by construction. Any other identity makes its own folder or stops.
pub(crate) fn may_adopt_legacy_folder(identity: &crate::identity::EngineIdentity) -> bool {
    identity.is_real()
}

/// The device-folder choice this identity may act on: every choice for the real app; for any other, never an
/// adoption (the pass stops with these words instead of writing its manifest into a folder it did not make).
pub(crate) fn permitted_choice(
    identity: &crate::identity::EngineIdentity,
    choice: beebeeb_core::backup::DeviceFolderChoice,
) -> Result<beebeeb_core::backup::DeviceFolderChoice, &'static str> {
    match choice {
        beebeeb_core::backup::DeviceFolderChoice::AdoptLegacy(_) if !may_adopt_legacy_folder(identity) => {
            Err("this build does not adopt a backup folder it did not make")
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_core::backup::{find_device_folder, DeviceFolderCandidate, DeviceFolderChoice, FolderManifest};

    fn legacy_folder(name: &str) -> DeviceFolderCandidate {
        DeviceFolderCandidate {
            folder_id: uuid::Uuid::parse_str("0193b1a6-0000-7000-8000-00000000a001").unwrap(),
            // Exactly what the old mirror named it (`<sync_root>/Backups/<sanitize_device_segment(name)>`).
            name: crate::known_folder::sanitize_device_segment(name),
            manifest: FolderManifest::Absent,
        }
    }

    /// The old mirror's `Backups/TestPC` is adopted, never doubled, when the device's name comes from today's
    /// source. The upper-case NetBIOS name (`GetComputerNameW`) would start `TESTPC (…)` next to it: the reason the
    /// source is fixed here.
    #[test]
    fn legacy_mirror_folder_is_adopted_with_todays_name_source() {
        let folder = legacy_folder("TestPC");
        let windows = |name: &str| DeviceSelf {
            device_id: beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0042-3510-8052-b4c04f4e3732"),
            name: name.to_string(),
            platform: Platform::Windows,
        };
        assert_eq!(
            find_device_folder(&windows("TestPC"), std::slice::from_ref(&folder)),
            DeviceFolderChoice::AdoptLegacy(folder.folder_id)
        );
        match find_device_folder(&windows("TESTPC"), std::slice::from_ref(&folder)) {
            DeviceFolderChoice::Create { name } => assert!(name.starts_with("TESTPC ("), "{name}"),
            other => panic!("the NetBIOS name is expected to double the folder, got {other:?}"),
        }

        // This machine: the folder the old mirror made here (named from `runner::device_name`, as the mirror does)
        // is adopted by the name `device_self` uses.
        let mine = legacy_folder(&crate::runner::device_name().expect("the test machine has a name"));
        let me = device_self_from(&crate::identity::REAL, backup_device_name().unwrap(), "test-machine").unwrap();
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

    /// The real app's device is today's, byte for byte: the id is core's digest of the bare machine id and the
    /// name is the computer's.
    #[test]
    fn the_real_apps_device_is_unchanged() {
        let me = device_self_from(&crate::identity::REAL, "TestPC".to_string(), "4c4c4544-0042-3510").unwrap();
        assert_eq!(
            me.device_id,
            beebeeb_core::backup::device_id_from_machine_id("4c4c4544-0042-3510")
        );
        assert_eq!(me.name, "TestPC");
        assert!(may_adopt_legacy_folder(&crate::identity::REAL));
    }

    /// The test build on the same computer is another device: another id, a name that says so, and neither the
    /// real device's folder (found by its manifest) nor the old mirror's (found by its bare name) is its own.
    #[test]
    fn the_test_build_is_another_device_and_takes_no_folder_of_the_real_one() {
        let (machine, name) = ("4c4c4544-0042-3510", "TestPC");
        let real = device_self_from(&crate::identity::REAL, name.to_string(), machine).unwrap();
        let test = device_self_from(&crate::identity::TEST, name.to_string(), machine).unwrap();
        assert_ne!(test.device_id, real.device_id);
        assert_eq!(
            test.device_id,
            beebeeb_core::backup::device_id_from_machine_id("io.beebeeb.app.test:4c4c4544-0042-3510")
        );
        assert_eq!(test.name, "TestPC (test)");
        assert!(!may_adopt_legacy_folder(&crate::identity::TEST));

        // The old mirror's folder under the bare name: the real app adopts it, the test build makes its own.
        let mirror = legacy_folder(name);
        let on_windows = |me: &DeviceSelf| DeviceSelf {
            platform: Platform::Windows,
            ..me.clone()
        };
        assert_eq!(
            find_device_folder(&on_windows(&real), std::slice::from_ref(&mirror)),
            DeviceFolderChoice::AdoptLegacy(mirror.folder_id)
        );
        match find_device_folder(&on_windows(&test), std::slice::from_ref(&mirror)) {
            DeviceFolderChoice::Create { name } => assert!(name.starts_with("TestPC (test)"), "{name}"),
            other => panic!("the test build must make its own folder, got {other:?}"),
        }
    }

    /// Even when core answers "adopt" (a manifest-less folder that happens to carry the test build's own name),
    /// the test build does not: the pass stops. The real app adopts, as before.
    #[test]
    fn only_the_real_app_acts_on_an_adoption() {
        let id = uuid::Uuid::parse_str("0193b1a6-0000-7000-8000-00000000a001").unwrap();
        let adopt = || DeviceFolderChoice::AdoptLegacy(id);
        assert_eq!(permitted_choice(&crate::identity::REAL, adopt()), Ok(adopt()));
        assert!(permitted_choice(&crate::identity::TEST, adopt()).is_err());
        for identity in [&crate::identity::REAL, &crate::identity::TEST] {
            assert_eq!(
                permitted_choice(identity, DeviceFolderChoice::Existing(id)),
                Ok(DeviceFolderChoice::Existing(id))
            );
            let create = || DeviceFolderChoice::Create { name: "PC".to_string() };
            assert_eq!(permitted_choice(identity, create()), Ok(create()));
        }
        // And core does answer "adopt" for such a folder, so the rule above is what stops it.
        let test = device_self_from(&crate::identity::TEST, "TestPC".to_string(), "m").unwrap();
        let named_like_the_test_build = legacy_folder("TestPC (test)");
        let on_windows = DeviceSelf {
            platform: Platform::Windows,
            ..test
        };
        if let DeviceFolderChoice::AdoptLegacy(found) =
            find_device_folder(&on_windows, std::slice::from_ref(&named_like_the_test_build))
        {
            assert!(permitted_choice(&crate::identity::TEST, DeviceFolderChoice::AdoptLegacy(found)).is_err());
        }
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
            "TestPC",
            "DESKTOP-4F2K9QZ",
            "laptop-of-alex",
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
        let me = device_self(&crate::identity::REAL).expect("a name and a machine id");
        assert_eq!(me.device_id, beebeeb_core::backup::device_id_from_machine_id(&id));
        assert_eq!(me.platform, Platform::Windows);
    }

    /// Native rung: IOKit gives this Mac's platform UUID, stable across calls, and a name and device exist.
    #[cfg(target_os = "macos")]
    #[test]
    fn platform_uuid_is_read_on_macos() {
        let id = machine_id().expect("IOPlatformUUID is readable");
        assert!(uuid::Uuid::parse_str(&id).is_ok(), "IOPlatformUUID is a UUID");
        assert_eq!(machine_id().as_deref(), Some(id.as_str()), "the same id every time");
        assert!(backup_device_name().is_some_and(|n| !n.is_empty()));
        let me = device_self(&crate::identity::REAL).expect("a name and a machine id");
        assert_eq!(me.device_id, beebeeb_core::backup::device_id_from_machine_id(&id));
        assert_eq!(me.platform, Platform::Macos);
        assert!(device_self(&crate::identity::TEST).is_some());
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
