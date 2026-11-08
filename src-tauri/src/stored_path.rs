//! Task 2413: the one rule for the path of a server row, and the one map from a stored path onto the sync root.
//!
//! A row the server lists is stored at its parent's stored path and its own name, '/'-joined. The state DB writes it
//! only when every segment is a name this host holds as exactly one entry: core's `path_safety::check_component_for`
//! with the host's rules ([`check_name_for`]). A row that fails is refused: nothing about it is written, so a row
//! stored before stays exactly as it was, and is never removed for it.
//!
//! The rules are the HOST's on purpose (core's `Rules::Host`, which core's docs name for sync): sync mirrors a tree
//! both ways, and a name that is an ordinary file name on Linux or macOS (`aux.c`, `10:30.txt`, `a\b`) must keep
//! round-tripping there. Rewriting a name (core's `sanitize_component`) is never used for a stored path: the stored
//! path is the key the local scan compares with the name on disk, so a rewritten one reads as another file.
//!
//! [`path_under`] maps a stored path onto the sync root for the Windows placeholder code. It applies the Windows rules
//! on every host, splits on both separators, and checks containment component by component.

use std::path::{Component, Path, PathBuf};

use beebeeb_core::path_safety::{UnsafeName, check_component_for};

/// The host a stored path is checked for: how long one name may be, and whether the Windows rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Host {
    /// NTFS: a name of at most 255 UTF-16 units, and the Windows rules.
    Windows,
    /// APFS: a name of at most 255 UTF-16 units (an emoji counts two), as on HFS+. Sources: the
    /// Apple Developer Forums thread "How to query APFS file name size limit? (NAME_MAX is wrong)" (thread 726970;
    /// APFS's own format allows longer names, 1022 bytes), and `task_2413_the_host_length_is_what_this_volume_holds`,
    /// which creates the names on the CI runner's volume.
    Mac,
    /// Linux and any other host: a name of at most 255 bytes (ext4, btrfs, tmpfs, XFS).
    Linux,
}

impl Host {
    /// The host this build runs on.
    pub(crate) const BUILD: Host = if cfg!(target_os = "windows") {
        Host::Windows
    } else if cfg!(target_os = "macos") {
        Host::Mac
    } else {
        Host::Linux
    };

    /// Whether the Windows rules apply.
    pub(crate) fn windows_rules(self) -> bool {
        self == Host::Windows
    }

    /// `Err` when `name` is longer than one name on this host may be, in the host's own unit.
    fn check_length(self, name: &str) -> Result<(), UnsafeName> {
        match self {
            Host::Windows | Host::Mac if name.encode_utf16().count() > MAX_NAME => {
                Err(UnsafeName("is longer than 255 UTF-16 units"))
            }
            Host::Linux if name.len() > MAX_NAME => Err(UnsafeName("is longer than 255 bytes")),
            _ => Ok(()),
        }
    }
}

/// The length limit of one name, in the host's unit ([`Host`]); also core's byte limit.
const MAX_NAME: usize = beebeeb_core::path_safety::MAX_COMPONENT_BYTES;

#[cfg(test)]
thread_local! {
    static TEST_HOST: std::cell::Cell<Option<Host>> = const { std::cell::Cell::new(None) };
}

/// The host whose rules a stored path meets here: the build's, unless a test runs a block as another host
/// ([`HostRules`]).
pub(crate) fn host() -> Host {
    #[cfg(test)]
    if let Some(host) = TEST_HOST.with(std::cell::Cell::get) {
        return host;
    }
    Host::BUILD
}

/// While alive, [`host`] answers another host on this thread, so a test on any build runs the store with that host's
/// rules. Dropping it restores the build's answer.
#[cfg(test)]
pub(crate) struct HostRules(());

#[cfg(test)]
impl HostRules {
    /// The store runs as `host`.
    pub(crate) fn of(host: Host) -> Self {
        TEST_HOST.with(|cell| cell.set(Some(host)));
        Self(())
    }

    /// The store runs with the Windows rules (`true`), or without them (`false`: the build's host, or Linux on a
    /// Windows build).
    pub(crate) fn windows(windows: bool) -> Self {
        Self::of(match (windows, Host::BUILD) {
            (true, _) => Host::Windows,
            (false, Host::Windows) => Host::Linux,
            (false, build) => build,
        })
    }
}

#[cfg(test)]
impl Drop for HostRules {
    fn drop(&mut self) {
        TEST_HOST.with(|cell| cell.set(None));
    }
}

/// What the state DB did with a server row (`StateDb::upsert_server_file_row`, `StateDb::upsert_shared_server_row`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRowWrite {
    /// Written, at this path.
    Written(String),
    /// Not written, because a segment of its path breaks this rule. A row stored before is exactly as it was.
    Refused(UnsafeName),
}

/// One name, a single segment of a stored path, on `host`.
///
/// Refused on every host: an empty name (or one only of invisible characters), `.` and `..`, a `/`, a NUL, a name
/// longer than the host holds ([`Host`]: 255 UTF-16 units on Windows and macOS, 255 bytes on Linux). With the Windows
/// rules also: a `\`, a drive prefix, a `:` anywhere, `< > " | ? *`, control and bidi-control characters, a trailing
/// dot or space, and a reserved device name (`CON`, `NUL`, `COM1.txt` ...).
///
/// The length is the host's alone; every other rule is core's `check_component_for`. Core also refuses a name over 255
/// bytes, which Windows and macOS hold (100 CJK characters are 300 bytes), so such a name gets core's checks before its
/// byte limit from core (core refuses it on that limit and nothing earlier) and the checks after it from
/// [`check_past_core_length`].
pub(crate) fn check_name_for(name: &str, host: Host) -> Result<(), UnsafeName> {
    host.check_length(name)?;
    let windows = host.windows_rules();
    if name.len() <= MAX_NAME {
        return check_component_for(name, false, windows);
    }
    match check_component_for(name, false, windows) {
        Err(refused) if refused == core_length_refusal() => check_past_core_length(name, windows),
        other => other,
    }
}

/// [`check_name_for`] on this host.
pub(crate) fn check_name(name: &str) -> Result<(), UnsafeName> {
    check_name_for(name, host())
}

/// What core answers for a name whose only fault is its length.
fn core_length_refusal() -> UnsafeName {
    check_component_for(&"a".repeat(MAX_NAME + 1), false, false).expect_err("core refuses a name over its byte limit")
}

/// Core's checks after its byte limit, in core's order and with its messages (`check_component_for`, core
/// 124389a8), for a name core refused on length alone. `task_2413_the_checks_past_the_length_are_cores` compares
/// them with core on every name core can judge whole.
fn check_past_core_length(name: &str, windows: bool) -> Result<(), UnsafeName> {
    if name.contains('\0') {
        return Err(UnsafeName("contains a NUL byte"));
    }
    if name.contains('/') {
        return Err(UnsafeName("contains a path separator"));
    }
    if !windows {
        return Ok(());
    }
    if name.contains('\\') {
        return Err(UnsafeName("contains a path separator"));
    }
    if name.chars().any(char::is_control) {
        return Err(UnsafeName("contains a control character"));
    }
    if name.chars().any(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return Err(UnsafeName("contains a bidirection-control character"));
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(UnsafeName("starts with a Windows drive prefix"));
    }
    if name.contains(':') {
        return Err(UnsafeName("contains ':' (drive prefix or alternate data stream)"));
    }
    if name.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err(UnsafeName("contains a character Windows does not allow"));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(UnsafeName("ends with a dot or space (Windows strips it)"));
    }
    // Core's device-name test reads the stem (before the first '.', trailing spaces trimmed), so core is asked about the
    // stem itself: it passes every check above as the name did (an empty stem, refused as empty, is no device name
    // either), so core refuses it as a device name exactly when it is one.
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let device = check_component_for("CON", false, true);
    if stem.len() <= MAX_NAME && check_component_for(stem, false, true) == device {
        return device;
    }
    Ok(())
}

/// A stored path: every '/'-separated segment passes [`check_name_for`]. Leading `/`s are the stored form some older
/// rows have (every reader trims them), so they are allowed; an empty path, a trailing `/` and a doubled `/` leave an
/// empty segment and are refused.
pub(crate) fn check_path_for(path: &str, host: Host) -> Result<(), UnsafeName> {
    path.trim_start_matches('/')
        .split('/')
        .try_for_each(|segment| check_name_for(segment, host))
}

/// [`check_path_for`] with this host's rules: the check the state DB runs before it writes a server row.
pub(crate) fn check_path(path: &str) -> Result<(), UnsafeName> {
    check_path_for(path, host())
}

/// The path a server row is stored at: `parent` (a stored path, "" at the top level) and the row's own `name`, which
/// must be ONE name ([`check_name_for`]); the state DB checks every segment of the result before it writes it. An
/// empty `name` (it did not resolve) gives "", a row that is stored but never placed, as before. The joined form is the
/// one the pull always wrote, so a row's stored path does not change with this check.
pub(crate) fn compose_for(parent: &str, name: &str, host: Host) -> Result<String, UnsafeName> {
    if name.is_empty() {
        return Ok(String::new());
    }
    check_name_for(name, host)?;
    Ok(if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    })
}

/// [`compose_for`] with this host's rules.
pub(crate) fn compose(parent: &str, name: &str) -> Result<String, UnsafeName> {
    compose_for(parent, name, host())
}

/// `rel_path` (a stored path) as a path under `root`, or `None`.
///
/// The Windows rules apply on every host (its production callers are Windows's placeholder code, and the same rules
/// everywhere let a Linux test show what Windows does): the path is split on both `/` and `\`, and every segment must
/// pass [`check_name_for`] as [`Host::Windows`]. Then the joined path must be `root` followed by exactly one plain component per
/// segment: containment by component, never by string prefix. Leading and trailing `/`s are trimmed first (older
/// rows); an empty path is `None`.
pub(crate) fn path_under(root: &Path, rel_path: &str) -> Option<PathBuf> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty() {
        return None;
    }
    let mut candidate = root.to_path_buf();
    let mut segments = 0usize;
    for segment in rel.split(['/', '\\']) {
        check_name_for(segment, Host::Windows).ok()?;
        candidate.push(segment);
        segments += 1;
    }
    let below = candidate.strip_prefix(root).ok()?;
    let mut plain = 0usize;
    for component in below.components() {
        if !matches!(component, Component::Normal(_)) {
            return None;
        }
        plain += 1;
    }
    (plain == segments).then_some(candidate)
}

/// The walk from `root` to `dest` a file write takes: `root` itself, every folder below it on the way, and `dest` when
/// it exists must not be a link. A link is std's `FileType::is_symlink`: a Unix symlink, and on Windows a reparse point
/// whose tag is a name surrogate (a symlink or a junction). A Cloud Files or OneDrive placeholder carries a tag that is
/// not a name surrogate, so it passes, as in core's backup walk. `dest` must be `root` followed by plain components;
/// the walk ends at the first entry that is not there yet (nothing below it can be a link). `PermissionDenied` when
/// refused.
#[cfg_attr(unix, allow(dead_code))]
pub(crate) fn refuse_links_below(root: &Path, dest: &Path) -> std::io::Result<()> {
    let deny = |message: &'static str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, message);
    let rel = dest
        .strip_prefix(root)
        .map_err(|_| deny("write destination is not within an allowed root"))?;
    // `Ok(true)` when `path` is there and is no link, `Ok(false)` when it is not there.
    let present = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(deny("write destination goes through a link")),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    };
    let mut at = root.to_path_buf();
    if !present(&at)? {
        return Ok(());
    }
    for component in rel.components() {
        let Component::Normal(name) = component else {
            return Err(deny("write destination has a non-normal path component"));
        };
        at.push(name);
        if !present(&at)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names each class of the rule covers, with whether the Linux and macOS rules and the Windows rules accept
    /// them.
    const NAMES: &[(&str, bool, bool)] = &[
        ("report.pdf", true, true),
        ("Notes 2026", true, true),
        ("élan.txt", true, true),
        (" leading space.txt", true, true),
        ("", false, false),
        (".", false, false),
        ("..", false, false),
        ("a/b", false, false),
        ("/top/level.txt", false, false),
        ("a\0b", false, false),
        ("a\\b", true, false),
        ("..\\outside.txt", true, false),
        ("\\\\server\\share", true, false),
        ("C:x", true, false),
        ("C:\\x", true, false),
        ("a:b", true, false),
        ("report.txt:stream", true, false),
        ("CON", true, false),
        ("nul.txt", true, false),
        ("COM1.txt", true, false),
        ("Lpt9", true, false),
        ("trailing.", true, false),
        ("trailing ", true, false),
        ("star*", true, false),
        ("bell\u{7}", true, false),
    ];

    #[test]
    fn task_2413_each_name_class_meets_the_host_rule() {
        let mut checked = 0;
        for (name, unix, windows) in NAMES {
            assert_eq!(check_name_for(name, Host::Linux).is_ok(), *unix, "{name:?} on Linux");
            assert_eq!(check_name_for(name, Host::Mac).is_ok(), *unix, "{name:?} on macOS");
            assert_eq!(check_name_for(name, Host::Windows).is_ok(), *windows, "{name:?} on Windows");
            checked += 1;
        }
        assert_eq!(checked, 25);
    }

    #[test]
    fn task_2413_compose_keeps_one_name_under_its_parent() {
        for host in [Host::Linux, Host::Mac, Host::Windows] {
            assert_eq!(compose_for("", "a.txt", host).as_deref(), Ok("a.txt"));
            assert_eq!(compose_for("docs", "a.txt", host).as_deref(), Ok("docs/a.txt"));
            assert_eq!(compose_for("docs/", "a.txt", host).as_deref(), Ok("docs/a.txt"));
            // An older parent's leading slash stays as the pull always wrote it.
            assert_eq!(compose_for("/docs", "a.txt", host).as_deref(), Ok("/docs/a.txt"));
            assert_eq!(compose_for("docs", "", host).as_deref(), Ok(""), "a name that did not resolve");
            for name in ["..", ".", "a/b", "/a", "a/", "../../x"] {
                assert!(compose_for("docs", name, host).is_err(), "{name:?} is not one name");
            }
            for path in ["../up/a.txt", "docs/../../a.txt", "docs//x/a.txt", "docs/", ""] {
                assert!(check_path_for(path, host).is_err(), "{path:?}");
            }
            assert_eq!(check_path_for("/docs/a.txt", host), Ok(()), "an older row's leading slash");
        }
        assert_eq!(compose_for("docs", "CON", Host::Linux).as_deref(), Ok("docs/CON"));
        assert_eq!(compose_for("docs", "CON", Host::Mac).as_deref(), Ok("docs/CON"));
        assert!(compose_for("docs", "CON", Host::Windows).is_err());
        assert!(check_path_for("a:b/x.txt", Host::Windows).is_err(), "a parent segment with the Windows rules");
        assert_eq!(check_path_for("a:b/x.txt", Host::Linux), Ok(()));
    }

    fn han(n: usize) -> String {
        "漢".repeat(n)
    }

    fn emoji(n: usize) -> String {
        "😀".repeat(n)
    }

    /// Core's 255-byte limit is Linux's. Windows and macOS hold 255 UTF-16 units, so a long CJK or emoji name stays one
    /// name there, whatever else it holds, and every other rule still applies to it.
    #[test]
    fn task_2413_a_long_name_meets_the_host_length_and_every_other_rule() {
        // About 100 UTF-16 units of CJK broken by spaces, and by `、`: over 255 bytes, within 255 units.
        let spaced = format!("{}報告", "第三四半期 計画 ".repeat(11));
        let commas = "東京、大阪、".repeat(17);
        assert_eq!((spaced.encode_utf16().count(), spaced.len()), (101, 259));
        assert_eq!((commas.encode_utf16().count(), commas.len()), (102, 306));
        let units_255 = format!("{}a", emoji(127));
        assert_eq!((units_255.encode_utf16().count(), units_255.len()), (255, 509));
        // (name, Linux, macOS, Windows)
        let cases: Vec<(String, bool, bool, bool)> = vec![
            (han(85), true, true, true),
            (han(100), false, true, true),
            (spaced.clone(), false, true, true),
            (commas.clone(), false, true, true),
            (emoji(80), false, true, true),
            (units_255.clone(), false, true, true),
            (emoji(128), false, false, false),
            (han(255), false, true, true),
            (han(256), false, false, false),
            ("a".repeat(300), false, false, false),
            // Every other rule, past core's byte limit:
            (format!("{}:x", han(100)), false, true, false),
            (format!("C:{}", han(100)), false, true, false),
            (format!("CON.{}", han(100)), false, true, false),
            (format!("aux  .{}", han(100)), false, true, false),
            (format!("{}.", han(100)), false, true, false),
            (format!("{} ", han(100)), false, true, false),
            (format!("{}\\x", han(100)), false, true, false),
            (format!("{}?", han(100)), false, true, false),
            (format!("{}\u{7}", han(90)), false, true, false),
            (format!("{}\u{202e}", han(100)), false, true, false),
            (format!("{}\0", han(100)), false, false, false),
            (format!("{}/x", han(100)), false, false, false),
            // Core's checks before its byte limit still run: a long name only of invisible characters, or a dot and
            // them, is no name.
            ("\u{200b}".repeat(100), false, false, false),
            (format!(".{}", "\u{200b}".repeat(100)), false, false, false),
            (format!("CONSOLE.{}", han(100)), false, true, true),
        ];
        for (name, linux, mac, windows) in &cases {
            let at = (name.encode_utf16().count(), name.len());
            assert_eq!(check_name_for(name, Host::Linux).is_ok(), *linux, "{at:?} (units, bytes) on Linux: {name:?}");
            assert_eq!(check_name_for(name, Host::Mac).is_ok(), *mac, "{at:?} (units, bytes) on macOS: {name:?}");
            assert_eq!(check_name_for(name, Host::Windows).is_ok(), *windows, "{at:?} (units, bytes) on Windows: {name:?}");
        }
        assert_eq!(cases.len(), 25);
        assert_eq!(check_name_for(&format!("{}:x", han(100)), Host::Windows), check_component_for("漢:x", false, true));
    }

    /// The checks [`check_name_for`] runs past core's byte limit are core's: on every name core judges whole (at most
    /// 255 bytes, past core's empty-name and dot-name checks) they answer exactly what core answers, message included.
    #[test]
    fn task_2413_the_checks_past_the_length_are_cores() {
        let mut names: Vec<String> = NAMES
            .iter()
            .map(|(name, _, _)| name.to_string())
            .filter(|name| !["", ".", ".."].contains(&name.as_str()))
            .collect();
        for name in [
            "a<b", "a>b", "a\"b", "a|b", "a?b", "a*b", "\u{202a}x", "x\u{2069}", "x\u{202d}y", "a\u{7f}", "a\u{85}b",
            "C:", "z:foo", "1:foo", "名前:x", "  ", "a.", "a ", "a. ", "CON..x", "COM10", "LPT", "CONSOLE", "COM⁴", "CON_",
            "XCON", "aux .txt", "nul   ", "con.", "e\u{301}", "\u{200b}x", "x\u{200f}", "a\tb", "plain name.txt",
        ] {
            names.push(name.to_string());
        }
        let mut stems: Vec<String> = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        stems.extend((0..10).flat_map(|n| [format!("COM{n}"), format!("LPT{n}")]));
        for stem in &stems {
            names.extend([stem.clone(), stem.to_lowercase(), format!("{stem}.txt"), format!("{stem} .tar.gz")]);
        }
        let mut compared = 0;
        let mut reached = std::collections::BTreeSet::new();
        for name in &names {
            assert!(name.len() <= MAX_NAME);
            for windows in [false, true] {
                let core = check_component_for(name, false, windows);
                assert_eq!(check_past_core_length(name, windows), core, "{name:?}, Windows rules: {windows}");
                compared += 1;
                if let Err(UnsafeName(message)) = core {
                    reached.insert(message);
                }
            }
        }
        assert_eq!(compared, 2 * names.len());
        assert_eq!(names.len(), 22 + 34 + 4 * 32);
        // The corpus reaches every refusal core makes past its byte limit.
        assert_eq!(reached.len(), 9, "{reached:?}");
    }

    /// The host length is what this build's volume holds: each name is created in a temporary folder, and the volume
    /// takes exactly the names [`check_name_for`] accepts for [`Host::BUILD`]. On the macOS CI runner this is APFS, on
    /// Windows NTFS, on Linux the runner's ext4 (or tmpfs). Every mismatch is listed with its length in each unit.
    #[test]
    fn task_2413_the_host_length_is_what_this_volume_holds() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "a".repeat(255),
            "a".repeat(256),
            han(85),
            han(86),
            han(255),
            han(256),
            format!("{}a", emoji(63)),
            emoji(64),
            emoji(80),
            format!("{}a", emoji(127)),
            emoji(128),
            "é".repeat(127),
            "é".repeat(128),
            "é".repeat(200),
            "é".repeat(255),
            "é".repeat(256),
            "e\u{301}".repeat(127),
            "e\u{301}".repeat(128),
        ];
        let mut mismatches = Vec::new();
        for name in &names {
            let held = std::fs::write(dir.path().join(name), b"x");
            let accepted = check_name_for(name, Host::BUILD);
            if held.is_ok() != accepted.is_ok() {
                mismatches.push(format!(
                    "{} chars, {} UTF-16 units, {} bytes: the volume {:?}, the rule {:?}",
                    name.chars().count(),
                    name.encode_utf16().count(),
                    name.len(),
                    held.map_err(|e| e.to_string()),
                    accepted
                ));
            }
        }
        assert!(mismatches.is_empty(), "{:?}: the volume and the rule differ on\n{}", Host::BUILD, mismatches.join("\n"));
        assert_eq!(names.len(), 18);
    }

    /// The walk refuses a link at the root, at a folder on the way and at the destination, and passes plain entries
    /// and a destination that is not there yet. (Windows: the junction test in `stored_path_2413_windows`.)
    #[cfg(unix)]
    #[test]
    fn task_2413_a_write_walk_refuses_a_link_anywhere_from_the_root_down() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("kept.txt"), b"kept").unwrap();
        std::fs::write(root.join("docs/own.txt"), b"own").unwrap();
        symlink(&outside, root.join("docs/link")).unwrap();
        symlink(outside.join("kept.txt"), root.join("docs/file-link.txt")).unwrap();
        symlink(&root, dir.path().join("root-link")).unwrap();
        let denied = |dest: &Path, base: &Path| {
            let error = refuse_links_below(base, dest).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{}: {error}", dest.display());
        };
        denied(&root.join("docs/link/kept.txt"), &root);
        denied(&root.join("docs/file-link.txt"), &root);
        denied(&dir.path().join("root-link/docs/own.txt"), &dir.path().join("root-link"));
        denied(&dir.path().join("elsewhere.txt"), &root);
        refuse_links_below(&root, &root.join("docs/own.txt")).unwrap();
        refuse_links_below(&root, &root.join("docs/new.txt")).unwrap();
        refuse_links_below(&root, &root.join("new-folder/new.txt")).unwrap();
    }

    #[test]
    fn task_2413_the_test_host_rules_end_with_their_guard() {
        assert_eq!(host(), Host::BUILD);
        for other in [Host::Windows, Host::Mac, Host::Linux] {
            {
                let _rules = HostRules::of(other);
                assert_eq!(host(), other);
            }
            assert_eq!(host(), Host::BUILD);
        }
        {
            let _rules = HostRules::windows(true);
            assert_eq!(host(), Host::Windows);
        }
        {
            let _rules = HostRules::windows(false);
            assert!(!host().windows_rules());
        }
        assert_eq!(host(), Host::BUILD);
    }
}
