//! OS words: the platform-specific nouns and shortcuts every sentence the app shows is built from.
//!
//! Windows output is the board text byte for byte. The macros expand to string literals, so they work in
//! `concat!` and `const`; the functions return the same literals.

#[cfg(target_os = "windows")]
macro_rules! os_lits {
    ($m:ident, $win:literal, $mac:literal, $other:literal) => {
        #[macro_export]
        macro_rules! $m {
            () => {
                $win
            };
        }
    };
}
#[cfg(target_os = "macos")]
macro_rules! os_lits {
    ($m:ident, $win:literal, $mac:literal, $other:literal) => {
        #[macro_export]
        macro_rules! $m {
            () => {
                $mac
            };
        }
    };
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
macro_rules! os_lits {
    ($m:ident, $win:literal, $mac:literal, $other:literal) => {
        #[macro_export]
        macro_rules! $m {
            () => {
                $other
            };
        }
    };
}

os_lits!(device_word, "this PC", "this Mac", "this computer");
os_lits!(device_word_cap, "This PC", "This Mac", "This computer");
os_lits!(file_manager, "File Explorer", "Finder", "Files");
os_lits!(unlock_name, "Windows Hello", "Touch ID", "");
os_lits!(primary_key, "Ctrl+", "⌘", "Ctrl+");
os_lits!(process_viewer, "Task Manager", "Activity Monitor", "your system monitor");

/// "this Mac" / "this PC" / "this computer".
pub const fn device_word() -> &'static str {
    device_word!()
}
/// "This Mac" / "This PC" / "This computer".
pub const fn device_word_cap() -> &'static str {
    device_word_cap!()
}
/// "Finder" / "File Explorer" / "Files".
pub const fn file_manager() -> &'static str {
    file_manager!()
}
/// "Touch ID" / "Windows Hello" / "" (no biometric unlock name on Linux).
pub const fn unlock_name() -> &'static str {
    unlock_name!()
}
/// "⌘" / "Ctrl+".
pub const fn primary_key() -> &'static str {
    primary_key!()
}

/// A shortcut in this OS's notation. `key` is the key with optional `Shift+` / `Alt+` prefixes
/// ("C", "Shift+N"): "⌘C" / "Ctrl+C", "⇧⌘N" / "Ctrl+Shift+N".
pub fn shortcut(key: &str) -> String {
    let mut rest = key;
    let (mut shift, mut alt) = (false, false);
    loop {
        if let Some(r) = rest.strip_prefix("Shift+") {
            shift = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Alt+") {
            alt = true;
            rest = r;
        } else {
            break;
        }
    }
    if cfg!(target_os = "macos") {
        // Apple order: ⌥ ⇧ ⌘ then the key.
        format!(
            "{}{}{}{}",
            if alt { "⌥" } else { "" },
            if shift { "⇧" } else { "" },
            primary_key(),
            rest
        )
    } else {
        format!(
            "{}{}{}{}",
            primary_key(),
            if alt { "Alt+" } else { "" },
            if shift { "Shift+" } else { "" },
            rest
        )
    }
}

/// Turn a Windows board sentence (goldens, FRAMES.md) into this OS's words.
#[cfg(any(test, feature = "test-support"))]
pub fn for_this_os(board_text: &str) -> String {
    let mut s = board_text.to_string();
    if cfg!(target_os = "macos") {
        s = s.replace("Ctrl+Alt+Shift+", "⌥⇧⌘");
        s = s.replace("Ctrl+Shift+", "⇧⌘");
        s = s.replace("Ctrl+Alt+", "⌥⌘");
    }
    s.replace("File Explorer", file_manager())
        .replace("Windows Hello", unlock_name())
        .replace("This PC", device_word_cap())
        .replace("this PC", device_word())
        .replace("Ctrl+", primary_key())
}

#[cfg(test)]
#[path = "os_words_tests.rs"]
mod tests;
