//! Spec 2026-10-10 §7.1 and §7.2 (task 1887): where the kept folder is, and what each kept file is called.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::path::{Path, PathBuf};

/// The folder's name under the container's `Documents` (§7.1).
pub const KEPT_FOLDER_NAME: &str = "Changes not sent";
/// `tauri.conf.json`'s `identifier`; a test pins the two equal.
pub const APP_BUNDLE_ID: &str = "io.beebeeb.app";
/// The longest file name APFS takes, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// §7.2: the newest copied write of a file gets the plain name; every other copied write (an earlier build's) is
/// `<stem> (earlier save <N>)<.ext>`, N = 1 for the next-newest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptRole {
    Newest,
    EarlierSave(u32),
}

/// §7.1: `<document_dir>/Changes not sent` only inside this app's sandbox container, where
/// `document_dir` is `<home>/Library/Containers/<bundle_id>/Data/Documents`; otherwise
/// `<data_dir>/beebeeb/Changes not sent`, so an unsandboxed build never writes plaintext into the person's real
/// Documents folder.
pub fn kept_root(document_dir: Option<&Path>, data_dir: Option<&Path>, bundle_id: &str) -> Option<PathBuf> {
    let container_documents = Path::new("Library")
        .join("Containers")
        .join(bundle_id)
        .join("Data")
        .join("Documents");
    match document_dir {
        Some(dir) if dir.is_absolute() && dir.ends_with(&container_documents) => Some(dir.join(KEPT_FOLDER_NAME)),
        _ => data_dir.map(|data| data.join("beebeeb").join(KEPT_FOLDER_NAME)),
    }
}

/// [`kept_root`] for this process.
// A test build keeps under its sandbox (`sign_out_kept_root` in `lib.rs`), so only a release build calls this.
#[cfg_attr(test, allow(dead_code))]
pub fn kept_root_for_this_app() -> Option<PathBuf> {
    kept_root(
        dirs::document_dir().as_deref(),
        dirs::data_dir().as_deref(),
        APP_BUNDLE_ID,
    )
}

/// §7.1: local time at the start of the keep step, as macOS names screenshots (dots, never colons).
pub fn event_folder_name(at: chrono::DateTime<chrono::Local>) -> String {
    at.format("%Y-%m-%d at %H.%M.%S").to_string()
}

/// `name` split before its extension. A name that starts with a dot and has no other dot has none.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => (&name[..dot], &name[dot..]),
    }
}

/// The longest prefix of `text` that fits in `max` bytes, cut at a character boundary.
fn prefix_within(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The bytes `text`'s first character takes; 0 for an empty text.
fn first_char_len(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

/// `name` cut to 255 bytes; the stem is cut, never the extension.
pub fn cut_name(name: &str) -> String {
    with_suffix(name, "")
}

/// `<stem><suffix><.ext>`, the stem cut so the whole is at most 255 bytes (§7.2 "Lengths": the cut applies to the
/// final name, never to the suffix, and never to the extension while it leaves room for the stem's first character
/// and the suffix). An "extension" that leaves no such room (`a.<250 bytes>`) is part of the stem, so the name is
/// cut as one stem and can never pass 255 bytes. The first character can take more than one byte, so the room is
/// measured for it, and a stem is never cut to nothing.
pub fn with_suffix(name: &str, suffix: &str) -> String {
    let (stem, extension) = match split_extension(name) {
        (stem, extension) if extension.len() + suffix.len() + first_char_len(stem) > MAX_NAME_BYTES => (name, ""),
        split => split,
    };
    let room = MAX_NAME_BYTES.saturating_sub(suffix.len() + extension.len());
    format!("{}{suffix}{extension}", prefix_within(stem, room))
}

/// §7.2 "Names": the components of a kept file's path under the event folder.
pub fn kept_relative_path(file_path: Option<&str>, op_id: &str, role: KeptRole) -> Vec<String> {
    let mut components: Vec<String> = file_path
        .unwrap_or_default()
        .split('/')
        .filter(|component| !component.is_empty())
        .map(|component| {
            if component == "." || component == ".." || component.contains('\0') {
                "_".to_string()
            } else {
                component.to_string()
            }
        })
        .collect();
    if components.is_empty() {
        components.push(format!("Unnamed {}", prefix_within(op_id, 8)));
    }
    let last = components.len() - 1;
    for (index, component) in components.iter_mut().enumerate() {
        *component = if index < last {
            prefix_within(component, MAX_NAME_BYTES).to_string()
        } else {
            match role {
                KeptRole::Newest => cut_name(component),
                KeptRole::EarlierSave(n) => with_suffix(component, &format!(" (earlier save {n})")),
            }
        };
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// U33.
    #[test]
    fn kept_root_is_the_container_or_app_support() {
        // `kept_root` demands an absolute container path; `/Users/a` is not absolute on Windows (no drive), so the
        // test's home carries one there. The logic under test is the same path-component match on both.
        let home = if cfg!(windows) { r"C:\Users\a" } else { "/Users/a" };
        let under_home = |rest: &str| Path::new(home).join(rest);
        let data = under_home("Library/Containers/io.beebeeb.app/Data/Library/Application Support");
        let data = data.as_path();
        let sandboxed = under_home("Library/Containers/io.beebeeb.app/Data/Documents");
        let sandboxed = sandboxed.as_path();
        assert_eq!(
            kept_root(Some(sandboxed), Some(data), APP_BUNDLE_ID),
            Some(sandboxed.join("Changes not sent"))
        );
        let fallback = Some(data.join("beebeeb").join("Changes not sent"));
        assert_eq!(
            kept_root(Some(under_home("Documents").as_path()), Some(data), APP_BUNDLE_ID),
            fallback,
            "unsandboxed"
        );
        let other = under_home("Library/Containers/com.example.other/Data/Documents");
        let other = other.as_path();
        assert_eq!(
            kept_root(Some(other), Some(data), APP_BUNDLE_ID),
            fallback,
            "another bundle's container"
        );
        assert_eq!(
            kept_root(
                Some(Path::new("Library/Containers/io.beebeeb.app/Data/Documents")),
                Some(data),
                APP_BUNDLE_ID
            ),
            fallback,
            "a relative path is not the container"
        );
        assert_eq!(kept_root(None, Some(data), APP_BUNDLE_ID), fallback);
        assert_eq!(kept_root(None, None, APP_BUNDLE_ID), None);
    }

    /// The bundle id the root is checked against is the app's own (`tauri.conf.json`).
    #[test]
    fn the_bundle_id_is_the_apps() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"].as_str(), Some(APP_BUNDLE_ID));
    }

    /// U11, the naming half; the writer's tests hold the disk half.
    #[test]
    fn kept_names_follow_the_rule() {
        let name = |path: Option<&str>, role| kept_relative_path(path, "0123456789abcdef", role);
        assert_eq!(
            name(Some("Work/report.docx"), KeptRole::Newest),
            ["Work", "report.docx"]
        );
        assert_eq!(
            name(Some("/Work//report.docx"), KeptRole::Newest),
            ["Work", "report.docx"],
            "empty components dropped"
        );
        assert_eq!(
            name(Some("Old/a.txt"), KeptRole::EarlierSave(1)),
            ["Old", "a (earlier save 1).txt"]
        );
        assert_eq!(
            name(Some("Old/a.txt"), KeptRole::EarlierSave(2)),
            ["Old", "a (earlier save 2).txt"]
        );
        assert_eq!(
            name(Some(".env"), KeptRole::EarlierSave(1)),
            [".env (earlier save 1)"],
            "a dotfile has no extension"
        );
        assert_eq!(name(Some("a/../b/./c"), KeptRole::Newest), ["a", "_", "b", "_", "c"]);
        assert_eq!(name(Some("a/b\0c"), KeptRole::Newest), ["a", "_"]);
        assert_eq!(name(None, KeptRole::Newest), ["Unnamed 01234567"]);
        assert_eq!(name(Some("/"), KeptRole::Newest), ["Unnamed 01234567"]);

        let long = format!("{}.docx", "x".repeat(300));
        let cut = name(Some(&long), KeptRole::Newest).pop().unwrap();
        assert_eq!(cut.len(), 255, "{cut}");
        assert!(cut.ends_with(".docx"), "{cut}");
        let accented = format!("{}.txt", "é".repeat(200)); // 2 bytes each
        let cut = name(Some(&accented), KeptRole::Newest).pop().unwrap();
        // A 400-byte stem gets 251 bytes, which ends inside a character, so it is cut back to 250.
        assert_eq!(cut.len(), 254, "{cut}");
        assert!(cut.ends_with(".txt"), "{cut}");

        let full = format!("{}.docx", "y".repeat(250)); // exactly 255 bytes
        let with = with_suffix(&full, " (2)");
        assert_eq!(with.len(), 255);
        assert!(with.ends_with(" (2).docx"), "{with}");

        // An "extension" that leaves no room for the suffix is cut as part of one stem, so no name
        // passes 255 bytes (`clonefile` would answer `ENAMETOOLONG` and fail the copy).
        let long_extension = format!("a.{}", "e".repeat(250)); // 252 bytes, the dot at byte 1
        let earlier = name(Some(&long_extension), KeptRole::EarlierSave(1)).pop().unwrap();
        assert!(
            earlier.len() <= 255 && earlier.ends_with(" (earlier save 1)"),
            "{} bytes: {earlier}",
            earlier.len()
        );
        let second = with_suffix(&long_extension, " (2)");
        assert!(
            second.len() <= 255 && second.ends_with(" (2)") && second.starts_with("a."),
            "{} bytes: {second}",
            second.len()
        );
        let over = format!("a.{}", "e".repeat(300)); // a name from another platform, longer than APFS takes
        assert_eq!(cut_name(&over).len(), 255);

        // An extension and suffix that leave less than the stem's first character are cut with the stem, so a name
        // never loses its whole stem (spec §7.2 "Lengths").
        let narrow = format!("é.{}", "e".repeat(249)); // a 250-byte extension: with " (2)", 254 bytes
        let second = with_suffix(&narrow, " (2)");
        assert!(
            second.len() <= 255 && second.starts_with("é.") && second.ends_with(" (2)"),
            "{} bytes: {second}",
            second.len()
        );
        let hidden = cut_name(&format!("é.{}", "e".repeat(253))); // a 254-byte extension
        assert!(
            hidden.len() <= 255 && hidden.starts_with("é."),
            "{} bytes: {hidden}",
            hidden.len()
        );
        let ascii = format!("a.{}", "e".repeat(249)); // the same 254 bytes leave room for a one-byte stem
        assert_eq!(with_suffix(&ascii, " (2)"), format!("a (2).{}", "e".repeat(249)));

        // A folder name longer than 255 bytes is cut too, at a character boundary; the file keeps its name.
        let dir = name(Some(&format!("{}/a.txt", "d".repeat(300))), KeptRole::Newest);
        assert_eq!(dir[0].len(), 255, "a long folder name: {}", dir[0]);
        assert_eq!(dir[1], "a.txt");
        let dir = name(Some(&format!("{}/a.txt", "é".repeat(150))), KeptRole::Newest); // 300 bytes
        assert_eq!(
            dir[0].len(),
            254,
            "a long folder name, cut at a character boundary: {}",
            dir[0]
        );
        assert_eq!(dir[1], "a.txt");
    }

    #[test]
    fn the_event_folder_reads_like_a_screenshot() {
        use chrono::TimeZone;
        let at = chrono::Local.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
        assert_eq!(event_folder_name(at), "2026-01-02 at 03.04.05");
    }
}
