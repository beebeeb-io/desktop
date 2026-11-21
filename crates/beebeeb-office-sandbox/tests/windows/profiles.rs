// The profiles (decision 2541), a child of `mod windows` in `tests/sandbox.rs`: moved here whole from the end of that
// file, so it stays under the 1,500-line limit for tests once the Office and media halves of it are merged.

use super::*;

// ── profiles (decision 2541): the system-decoder profile adds registry reads and COM, and nothing else ───────

const EVERY_PROFILE: [Profile; 3] = [
    Profile::NoCapabilities,
    Profile::SystemDecoder { hevc: false },
    Profile::SystemDecoder { hevc: true },
];

/// A loopback connection refused: before any socket exists (no capability, measured), or by the system at the
/// connection (`registryRead` with `lpacCom`, and the five HEVC capabilities: 10013, measured by the media spike).
/// `registryRead` with `lpacCom` alone (no HEVC) was not measured on the network: one of the two, and the listener
/// counts nothing.
const SYSTEM_DECODER_LOOPBACK_REFUSED: [&str; 2] = [NETWORK_REFUSED, "err 10013"];

/// Keys every Windows has: the machine's, the classes' and the user's environment (the spike opened these too).
const REGISTRY_KEYS: [(&str, &str); 3] = [
    ("hklm", r"SOFTWARE\Microsoft\Windows\CurrentVersion"),
    ("hkcr", "CLSID"),
    ("hkcu", "Environment"),
];

/// The attack probe against every profile: reading a file of the user, writing anywhere the user can, a
/// loopback connection and starting a process are refused in each, exactly as in the default profile.
#[test]
fn every_profile_denies_file_read_write_loopback_and_process_start() {
    let _serial = serial();
    let places = user_places();
    assert!(
        places.len() >= 4,
        "temp, the profile and at least two more: {}",
        places.len()
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let loopback = listener.local_addr().unwrap();
    let system_root = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
    let shell = system_root.join("System32").join("cmd.exe");
    let shell = shell.to_str().unwrap();

    // The instruments can fail: the plain probe reads, writes, connects and starts a program.
    let mut plain = Probe::plain();
    for place in &places {
        let canary = Scratch::in_dir(place);
        std::fs::write(&canary.0, b"canary").unwrap();
        assert_eq!(plain.attempt(&format!("read {}", canary.text())), "ok 6");
        let control = Scratch::in_dir(place);
        assert_eq!(plain.attempt(&format!("write {}", control.text())), "ok");
        assert!(control.0.exists(), "the plain probe created its file");
    }
    assert_eq!(plain.attempt(&format!("connect {loopback}")), "ok");
    assert_eq!(accepted(&listener), 1, "the listener counts a connection");
    assert_eq!(plain.attempt(&format!("spawn {shell}")), "ok");
    drop(plain);

    let mut refused = 0;
    for profile in EVERY_PROFILE {
        let mut confined = Probe::confined_as(profile);
        for place in &places {
            let canary = Scratch::in_dir(place);
            std::fs::write(&canary.0, b"canary").unwrap();
            assert_eq!(
                confined.attempt(&format!("read {}", canary.text())),
                ACCESS_DENIED,
                "{profile:?}: read a file of the user"
            );
            let target = Scratch::in_dir(place);
            assert_eq!(
                confined.attempt(&format!("write {}", target.text())),
                ACCESS_DENIED,
                "{profile:?}: write"
            );
            assert!(!target.0.exists(), "{profile:?}: nothing was created");
            refused += 2;
        }
        let answer = confined.attempt(&format!("connect {loopback}"));
        let refusals: &[&str] = if profile == Profile::NoCapabilities {
            &[NETWORK_REFUSED]
        } else {
            &SYSTEM_DECODER_LOOPBACK_REFUSED
        };
        assert!(
            refusals.contains(&answer.as_str()),
            "{profile:?}: loopback answered {answer}"
        );
        assert_eq!(accepted(&listener), 0, "{profile:?}: the listener counts no connection");
        assert_eq!(
            confined.attempt(&format!("spawn {shell}")),
            PROCESS_QUOTA,
            "{profile:?}: start a program"
        );
        assert_eq!(
            confined.attempt(&format!("spawn {PROBE}")),
            ACCESS_DENIED,
            "{profile:?}: start its own program"
        );
        refused += 3;
    }
    assert_eq!(
        refused,
        EVERY_PROFILE.len() * (2 * places.len() + 3),
        "every attempt of every profile was made"
    );
}

/// The one thing the system-decoder profile adds: reading the registry. The default profile cannot.
#[test]
fn only_the_system_decoder_profile_reads_the_registry() {
    let _serial = serial();
    let mut plain = Probe::plain();
    for (hive, key) in REGISTRY_KEYS {
        assert_eq!(
            plain.attempt(&format!("read-registry {hive} {key}")),
            "ok",
            "the instrument can fail: the plain probe opens the key ({hive})"
        );
    }
    drop(plain);

    let mut counted = 0;
    for profile in EVERY_PROFILE {
        let expected = if profile == Profile::NoCapabilities {
            ACCESS_DENIED
        } else {
            "ok"
        };
        let mut confined = Probe::confined_as(profile);
        for (hive, key) in REGISTRY_KEYS {
            assert_eq!(
                confined.attempt(&format!("read-registry {hive} {key}")),
                expected,
                "{profile:?}: {hive}"
            );
            counted += 1;
        }
    }
    assert_eq!(counted, EVERY_PROFILE.len() * REGISTRY_KEYS.len());
}

/// A security identifier as text, the same way the probe writes it: `S-<revision>-<authority>-<part>...`.
fn identifier_text(sid: *mut c_void) -> String {
    // SAFETY: `sid` is a valid identifier the system derived: an 8-byte head (its byte 1 is the part count),
    // then 4 bytes per part.
    let bytes = unsafe {
        let parts = usize::from(*sid.cast::<u8>().add(1));
        std::slice::from_raw_parts(sid.cast::<u8>(), 8 + 4 * parts)
    };
    let authority = bytes[2..8]
        .iter()
        .fold(0u64, |value, &byte| (value << 8) | u64::from(byte));
    let mut text = format!("S-{}-{authority}", bytes[0]);
    for part in bytes[8..].as_chunks::<4>().0 {
        text.push_str(&format!(
            "-{}",
            u32::from_le_bytes([part[0], part[1], part[2], part[3]])
        ));
    }
    text
}

/// The identifiers Windows derives from capability names, sorted: what a token of that profile must hold.
fn derived_identifiers(names: &[&str]) -> Vec<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::{DeriveCapabilitySidsFromName, PSID};
    let mut identifiers = Vec::new();
    for name in names {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut groups: *mut PSID = std::ptr::null_mut();
        let mut group_count = 0u32;
        let mut capabilities: *mut PSID = std::ptr::null_mut();
        let mut capability_count = 0u32;
        // SAFETY: a terminated UTF-16 name; the four out-pointers are valid places for the answers.
        let ok = unsafe {
            DeriveCapabilitySidsFromName(
                wide.as_ptr(),
                &mut groups,
                &mut group_count,
                &mut capabilities,
                &mut capability_count,
            )
        };
        assert!(ok != 0 && capability_count >= 1, "{name} gives an identifier");
        // SAFETY: the system returned arrays of these lengths; the first capability identifier is the one the
        // crate grants; every allocation is freed once afterwards.
        unsafe {
            identifiers.push(identifier_text(*capabilities));
            for index in 0..group_count as usize {
                LocalFree(*groups.add(index));
            }
            for index in 0..capability_count as usize {
                LocalFree(*capabilities.add(index));
            }
            LocalFree(groups.cast());
            LocalFree(capabilities.cast());
        }
    }
    identifiers.sort();
    identifiers
}

/// The helper's token, as the helper itself reads it, holds exactly the identifiers of its profile's names: no
/// more (an extra capability is red), no fewer, none twice.
#[test]
fn the_token_holds_exactly_the_profiles_capabilities() {
    let _serial = serial();
    let mut counts = Vec::new();
    for profile in EVERY_PROFILE {
        let wanted = derived_identifiers(profile.capabilities());
        let mut distinct = wanted.clone();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            profile.capabilities().len(),
            "{profile:?}: each name has an identifier of its own"
        );
        let mut confined = Probe::confined_as(profile);
        assert_eq!(
            confined.attempt("capabilities"),
            // The probe's line ends in a space when the list is empty; answers are read trimmed.
            format!("capabilities {} {}", wanted.len(), wanted.join(",")).trim_end(),
            "{profile:?}"
        );
        counts.push(wanted.len());
    }
    // The instrument tells the three profiles apart.
    assert_eq!(counts, [0, 2, 5]);
}

/// Windows' decoders start in the system-decoder profile, with and without HEVC: COM (`lpacCom`, Guus 2026-11-21),
/// Media Foundation, Microsoft's AAC decoder (a video's sound) and WIC (HEIC and camera RAW pictures). The default
/// profile cannot start them. The plain probe first: the instrument can say `ok`.
/// Run it in a desktop session: in session 0 `ole32.dll` does not initialise in any container (1114, measured).
#[test]
fn windows_decoders_start_in_the_system_decoder_profile_only() {
    let _serial = serial();
    let mut plain = Probe::plain();
    assert_eq!(
        plain.attempt("decoders"),
        "ok",
        "the instrument can say ok: the plain probe starts them"
    );
    drop(plain);
    let answers: Vec<String> = EVERY_PROFILE
        .iter()
        .map(|profile| Probe::confined_as(*profile).attempt("decoders"))
        .collect();
    assert!(answers[0].starts_with("err "), "NoCapabilities: {}", answers[0]);
    assert_eq!(
        answers[1..],
        ["ok", "ok"],
        "SystemDecoder without and with HEVC: {answers:?}"
    );
}
