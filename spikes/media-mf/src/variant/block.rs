//! The two text blocks a new process is given: its command line and its environment. Pure functions, so their
//! rules are tested on every operating system even though only the Windows module calls them.
#![cfg_attr(not(windows), allow(dead_code))]

use super::SandboxError;

/// The helper's whole environment, by name. Nothing is copied from the parent's environment.
///
/// - `SystemRoot` is needed by the system libraries a process loads at start; its value is asked from the system.
/// - `LOCALAPPDATA` has to be present or Windows refuses to create an AppContainer process (system code 203,
///   measured). Its value here is a placeholder (the system folder). Windows rewrites it and adds `TEMP` and
///   `TMP`, building all three from that placeholder plus the container's name (measured: each sits under the
///   placeholder, holds the profile name, and does not exist). So the user's profile path does not reach the
///   helper through its environment.
pub(crate) const ENVIRONMENT_NAMES: &[&str] = &["LOCALAPPDATA", "SystemRoot", "TEMP", "TMP"];

/// Every container profile of ours is named this prefix plus 32 lowercase hexadecimal digits.
// SPIKE: its own prefix, so neither the product nor the spike tidies up the other's profiles.
pub(crate) const PROFILE_PREFIX: &str = "beebeeb.mfspike.";

/// The profile name for 128 random bits.
pub(crate) fn profile_name(random: [u8; 16]) -> String {
    let mut name = String::from(PROFILE_PREFIX);
    for byte in random {
        name.push_str(&format!("{byte:02x}"));
    }
    name
}

/// True only for a name [`profile_name`] can produce. The clean-up of left-over profiles deletes nothing else.
pub(crate) fn is_our_profile_name(units: &[u16]) -> bool {
    let prefix: Vec<u16> = PROFILE_PREFIX.encode_utf16().collect();
    let Some(digits) = units.strip_prefix(prefix.as_slice()) else {
        return false;
    };
    let hex = |u: &u16| (b'0' as u16..=b'9' as u16).contains(u) || (b'a' as u16..=b'f' as u16).contains(u);
    digits.len() == 32 && digits.iter().all(hex)
}

/// Checks a helper path (as UTF-16 units, no terminator) and returns it with a terminator.
///
/// Refused: an empty path, one that is not `X:\…` (so nothing is searched and no device or network path is
/// taken), and one holding a NUL or a double quote (so the command line below cannot be split differently).
pub(crate) fn checked_path(units: &[u16]) -> Result<Vec<u16>, SandboxError> {
    const COLON: u16 = b':' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;
    if units.is_empty() {
        return Err(SandboxError::HelperPath("empty"));
    }
    let drive_letter = |u: u16| (b'A' as u16..=b'Z' as u16).contains(&u) || (b'a' as u16..=b'z' as u16).contains(&u);
    if units.len() < 4 || !drive_letter(units[0]) || units[1] != COLON || units[2] != BACKSLASH {
        return Err(SandboxError::HelperPath("not an absolute path on a drive"));
    }
    if units.iter().any(|&u| u == 0 || u == QUOTE) {
        return Err(SandboxError::HelperPath("holds a NUL or a quote"));
    }
    let mut out = units.to_vec();
    out.push(0);
    Ok(out)
}

/// The command line: the quoted program path and nothing else. `path` is a result of [`checked_path`].
pub(crate) fn command_line(path: &[u16]) -> Vec<u16> {
    let body = path.strip_suffix(&[0]).unwrap_or(path);
    let mut out = Vec::with_capacity(body.len() + 3);
    out.push(b'"' as u16);
    out.extend_from_slice(body);
    out.push(b'"' as u16);
    out.push(0);
    out
}

/// The environment block handed to process creation: `LOCALAPPDATA` (a placeholder Windows replaces) and
/// `SystemRoot`, both `<system_root>`, and the block terminator. `system_root` has no terminator.
pub(crate) fn environment_block(system_root: &[u16]) -> Result<Vec<u16>, SandboxError> {
    if system_root.is_empty() || system_root.iter().any(|&u| u == 0 || u == b'=' as u16) {
        return Err(SandboxError::Os {
            step: "system directory",
            code: 0,
        });
    }
    let mut out = Vec::new();
    for name in ["LOCALAPPDATA=", "SystemRoot="] {
        out.extend(name.encode_utf16());
        out.extend_from_slice(system_root);
        out.push(0);
    }
    out.push(0);
    Ok(out)
}
