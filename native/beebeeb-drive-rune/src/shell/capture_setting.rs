//! "Hide Beebeeb from screen capture and remote desktop": one per-PC switch, off by default.
//!
//! Off, every window can be seen in screenshots, recordings, screen shares and remote desktop
//! (RustDesk and the like), on every identity. On, every window asks the platform to leave itself out
//! of them (`Platform::set_capture_exclusion`). Stored in `<state dir>/native-capture.json`; a missing,
//! oversized or unreadable file is "off".

use std::path::Path;

pub const FILE_NAME: &str = "native-capture.json";
const KEY: &str = "hide_from_capture";
const MAX_FILE_BYTES: u64 = 4 * 1024;

pub fn load(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_BYTES => {}
        _ => return false,
    }
    std::fs::read_to_string(path).map(|text| from_json(&text)).unwrap_or(false)
}

pub fn save(path: &Path, hide: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(hide))?;
    std::fs::rename(&tmp, path)
}

fn to_json(hide: bool) -> String {
    format!("{{\"{KEY}\":{hide}}}\n")
}

fn from_json(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains(&format!("\"{KEY}\":true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_unless_the_file_says_on() {
        let dir = std::env::temp_dir().join(format!("bb-capture-setting-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        assert!(!load(&path), "no file: off");
        save(&path, true).unwrap();
        assert!(load(&path));
        save(&path, false).unwrap();
        assert!(!load(&path));
        std::fs::write(&path, "garbage").unwrap();
        assert!(!load(&path), "garbage: off");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
