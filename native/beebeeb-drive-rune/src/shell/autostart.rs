//! Start at login: the pure half. The registry writes are in `platform/windows`.
//!
//! The 0.8.x app's setting carries over because this is the same value in the same key that
//! `tauri-plugin-autostart` 2.5 (through `auto-launch` 0.5) writes:
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Beebeeb` (Tauri's
//! `package_info().name`, which is `productName` in `tauri.conf.json`).
//!
//! Two differences from what the plugin wrote, both on purpose:
//! - the path is quoted. The plugin writes it bare, so a path with a space (`C:\Program Files\…`)
//!   depends on how Windows guesses where the program name ends;
//! - `--autostart` follows it, so a start at login stays in the notification area instead of
//!   opening the window.
//!
//! [`needs_rewrite`] says when a value the old app left behind should be replaced by ours.

/// Under `HKEY_CURRENT_USER`.
pub const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Task Manager's "Startup apps" switch: a 12-byte value, first byte 2 or 6 for on, 3 for off.
pub const STARTUP_APPROVED_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
pub const VALUE_NAME: &str = "Beebeeb";
/// What a start at login passes, so the app knows to stay in the notification area.
pub const AUTOSTART_FLAG: &str = "--autostart";
/// What `auto-launch` writes to mark the entry enabled in Task Manager.
pub const STARTUP_APPROVED_ENABLED: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// One argument, quoted the way `CommandLineToArgvW` reads it back: wrapped in quotes when it
/// is empty or has a space, tab or quote; backslashes doubled only before a quote.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                backslashes = 0;
                out.push('"');
                continue;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
            }
        }
        if c != '\\' {
            out.push(c);
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// The Run value for this executable. The path is always quoted, with or without a space.
pub fn command_line(exe: &str) -> String {
    let exe = quote_arg(exe);
    let exe = if exe.starts_with('"') {
        exe
    } else {
        format!("\"{exe}\"")
    };
    format!("{exe} {AUTOSTART_FLAG}")
}

/// Whether the entry is on: the Run value exists, and Task Manager has not switched it off
/// (`auto-launch`'s reading: no approval value, or one whose last eight bytes are zero).
pub fn is_enabled(run_value: Option<&str>, startup_approved: Option<&[u8]>) -> bool {
    let approved = match startup_approved {
        Some(bytes) if bytes.len() >= 8 => bytes[bytes.len() - 8..].iter().all(|b| *b == 0),
        _ => true,
    };
    run_value.is_some() && approved
}

/// Whether an existing Run value should be replaced by [`command_line`]: it is on, but it is
/// the old app's bare path, or points at another install path.
pub fn needs_rewrite(run_value: Option<&str>, exe: &str) -> bool {
    run_value.is_some_and(|value| value != command_line(exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_quotes_the_path_and_says_autostart() {
        assert_eq!(
            command_line("C:\\Program Files\\Beebeeb\\Beebeeb.exe"),
            "\"C:\\Program Files\\Beebeeb\\Beebeeb.exe\" --autostart"
        );
        // Quoted also without a space, so the value has one shape.
        assert_eq!(
            command_line("C:\\Users\\anna\\AppData\\Local\\Beebeeb\\Beebeeb.exe"),
            "\"C:\\Users\\anna\\AppData\\Local\\Beebeeb\\Beebeeb.exe\" --autostart"
        );
    }

    #[test]
    fn quoting_follows_the_windows_argument_rules() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
        // A trailing backslash before the closing quote is doubled; one inside is not.
        assert_eq!(quote_arg("C:\\dir with space\\"), "\"C:\\dir with space\\\\\"");
        assert_eq!(quote_arg("a\\\\\"b c"), "\"a\\\\\\\\\\\"b c\"");
    }

    #[test]
    fn the_key_and_value_are_the_ones_the_tauri_plugin_writes() {
        assert_eq!(RUN_KEY, "Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        assert_eq!(VALUE_NAME, "Beebeeb");
    }

    #[test]
    fn enabled_means_present_and_not_switched_off_in_task_manager() {
        assert!(!is_enabled(None, None));
        assert!(is_enabled(Some("x"), None));
        assert!(is_enabled(Some("x"), Some(&STARTUP_APPROVED_ENABLED)));
        // Task Manager's "Disabled": 03 then a timestamp.
        assert!(!is_enabled(
            Some("x"),
            Some(&[0x03, 0, 0, 0, 0x10, 0x5a, 0x3c, 0x11, 0x2e, 0xd9, 0x01, 0])
        ));
        // Too short to read: treated as no opinion, as auto-launch does.
        assert!(is_enabled(Some("x"), Some(&[0x03])));
        assert!(!is_enabled(None, Some(&STARTUP_APPROVED_ENABLED)));
    }

    #[test]
    fn the_old_apps_value_is_rewritten_and_ours_is_left_alone() {
        let exe = "C:\\Program Files\\Beebeeb\\Beebeeb.exe";
        // What tauri-plugin-autostart wrote: the bare path and a trailing space.
        assert!(needs_rewrite(Some("C:\\Program Files\\Beebeeb\\Beebeeb.exe "), exe));
        assert!(needs_rewrite(Some("\"D:\\Old\\Beebeeb.exe\" --autostart"), exe));
        assert!(!needs_rewrite(Some(&command_line(exe)), exe));
        // Off stays off: nothing to rewrite.
        assert!(!needs_rewrite(None, exe));
    }
}
