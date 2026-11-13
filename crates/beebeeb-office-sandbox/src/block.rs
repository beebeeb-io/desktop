//! The two text blocks a new process is given: its command line and its environment. Pure functions, so their
//! rules are tested on every operating system even though only the Windows module calls them.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::SandboxError;

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
pub(crate) const PROFILE_PREFIX: &str = "beebeeb.office.";

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

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn only_a_name_of_our_own_shape_is_ours() {
        assert!(is_our_profile_name(&w(&profile_name([0xAB; 16]))));
        assert!(is_our_profile_name(&w(&profile_name([0; 16]))));
        let foreign = [
            "",
            "beebeeb.office.",
            "beebeeb.office.abababababababababababababababab0",
            "beebeeb.office.ababababababababababababababababab",
            "beebeeb.office.ABABABABABABABABABABABABABABABAB",
            "beebeeb.office.abababababababababababababababag",
            "beebeeb.officex.abababababababababababababababab",
            "xbeebeeb.office.abababababababababababababababab",
            "Microsoft.Windows.Something_8wekyb3d8bbwe",
        ];
        let mut refused = 0;
        for name in foreign {
            assert!(!is_our_profile_name(&w(name)), "case {refused}");
            refused += 1;
        }
        assert_eq!(refused, 9);
    }

    #[test]
    fn an_absolute_drive_path_is_taken_and_terminated() {
        let got = checked_path(&w(r"C:\Program Files\App\helper.exe")).unwrap();
        assert_eq!(got.last(), Some(&0));
        assert_eq!(got.len(), r"C:\Program Files\App\helper.exe".len() + 1);
    }

    #[test]
    fn a_path_that_would_be_searched_or_reach_another_machine_is_refused() {
        let refused = [
            "",
            "helper.exe",
            r".\helper.exe",
            r"\helper.exe",
            r"\\host\share\helper.exe",
            r"\\?\C:\helper.exe",
            "C:helper.exe",
            r"C:\",
        ];
        let mut count = 0;
        for path in refused {
            assert!(
                matches!(checked_path(&w(path)), Err(SandboxError::HelperPath(_))),
                "case {count}"
            );
            count += 1;
        }
        assert_eq!(count, 8);
    }

    #[test]
    fn a_path_that_could_split_the_command_line_is_refused() {
        assert_eq!(
            checked_path(&w("C:\\a\" --flag \"b.exe")),
            Err(SandboxError::HelperPath("holds a NUL or a quote"))
        );
        assert_eq!(
            checked_path(&w("C:\\a\0b.exe")),
            Err(SandboxError::HelperPath("holds a NUL or a quote"))
        );
    }

    #[test]
    fn the_command_line_is_the_quoted_path_and_nothing_else() {
        let path = checked_path(&w(r"C:\Program Files\App\helper.exe")).unwrap();
        assert_eq!(
            String::from_utf16(&command_line(&path)).unwrap(),
            "\"C:\\Program Files\\App\\helper.exe\"\0"
        );
    }

    #[test]
    fn the_environment_block_holds_two_variables_and_nothing_of_the_parent() {
        let block = environment_block(&w(r"C:\Windows")).unwrap();
        assert_eq!(
            String::from_utf16(&block).unwrap(),
            "LOCALAPPDATA=C:\\Windows\0SystemRoot=C:\\Windows\0\0"
        );
        assert_eq!(
            block.iter().filter(|&&u| u == 0).count(),
            3,
            "two entries, then the end of the block"
        );
        assert_eq!(ENVIRONMENT_NAMES, ["LOCALAPPDATA", "SystemRoot", "TEMP", "TMP"]);
    }

    #[test]
    fn a_system_directory_that_could_add_a_variable_is_refused() {
        assert!(environment_block(&w("C:\\Windows\0X=1")).is_err());
        assert!(environment_block(&w("C:\\Win=dows")).is_err());
        assert!(environment_block(&[]).is_err());
    }
}
