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

use super::identity::AppIdentity;

/// Under `HKEY_CURRENT_USER`.
pub const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// Task Manager's "Startup apps" switch: a 12-byte value, first byte 2 or 6 for on, 3 for off.
pub const STARTUP_APPROVED_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
/// The value's name: this identity's (`"Beebeeb"` for the real app, as the plugin wrote it).
pub const fn value_name(identity: AppIdentity) -> &'static str {
    identity.run_value_name()
}
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
    format!("{} {AUTOSTART_FLAG}", quote_arg_always(exe))
}

/// [`quote_arg`], in quotes also when the argument needs none.
pub fn quote_arg_always(arg: &str) -> String {
    let quoted = quote_arg(arg);
    if quoted.starts_with('"') {
        quoted
    } else {
        format!("\"{quoted}\"")
    }
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

/// Whether a Run value starts the program at `exe`, in either shape: ours (quoted) or the
/// 0.8.x plugin's (bare, also when the path has spaces). Windows paths compare without case.
pub fn points_at(run_value: &str, exe: &str) -> bool {
    let value = run_value.trim();
    let program = match value.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        None => {
            // Bare: the path is the whole value, or everything before the first " -" argument.
            let bytes = value.as_bytes();
            let end = (0..bytes.len())
                .find(|i| value[*i..].starts_with(" -"))
                .unwrap_or(bytes.len());
            value[..end].trim_end()
        }
    };
    !exe.is_empty() && program.eq_ignore_ascii_case(exe)
}

/// Whether an existing Run value should be replaced by [`command_line`]: it starts THIS
/// program, but in the old app's shape (a bare path, no `--autostart`). A value that starts
/// another program (another install, a development build elsewhere) is not ours to change,
/// and "off" stays off.
pub fn needs_rewrite(run_value: Option<&str>, exe: &str) -> bool {
    run_value.is_some_and(|value| value != command_line(exe) && points_at(value, exe))
}

/// What the person, or the app at start, wants of the Run value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wish {
    /// The Settings switch, on.
    Enable,
    /// The Settings switch, off.
    Disable,
    /// At start: bring the 0.8.x app's entry FOR THIS PROGRAM into this app's shape.
    Adopt,
}

/// The one registry change a wish may make to this identity's Run value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunChange {
    Write(String),
    Delete,
    Leave,
}

/// The value exists and starts a program at another path: another install of this identity
/// (the installed app, seen from a development build; an old copy). It is not this program's
/// to overwrite or delete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedByAnotherInstall;

/// Decides the change. `existing`: this identity's Run value as it is now; `exe`: this
/// program's own path. Whatever the wish, a value that starts another program is never
/// written over and never deleted.
pub fn plan(wish: Wish, existing: Option<&str>, exe: &str) -> Result<RunChange, OwnedByAnotherInstall> {
    if exe.is_empty() {
        return Ok(RunChange::Leave);
    }
    let ours = existing.map(|value| points_at(value, exe));
    match (wish, ours) {
        (Wish::Enable, None) => Ok(RunChange::Write(command_line(exe))),
        (Wish::Enable, Some(false)) => Err(OwnedByAnotherInstall),
        (Wish::Enable | Wish::Adopt, Some(true)) => {
            if existing == Some(command_line(exe).as_str()) {
                Ok(RunChange::Leave)
            } else {
                Ok(RunChange::Write(command_line(exe)))
            }
        }
        (Wish::Disable, Some(true)) => Ok(RunChange::Delete),
        (Wish::Disable, _) | (Wish::Adopt, _) => Ok(RunChange::Leave),
    }
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
            command_line("C:\\home\\anna\\AppData\\Local\\Beebeeb\\Beebeeb.exe"),
            "\"C:\\home\\anna\\AppData\\Local\\Beebeeb\\Beebeeb.exe\" --autostart"
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
        assert_eq!(value_name(AppIdentity::Real), "Beebeeb");
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
    fn the_old_apps_value_for_this_program_is_rewritten_and_nothing_else() {
        let exe = "C:\\Program Files\\Beebeeb\\Beebeeb.exe";
        // What tauri-plugin-autostart wrote: the bare path and a trailing space.
        assert!(needs_rewrite(Some("C:\\Program Files\\Beebeeb\\Beebeeb.exe "), exe));
        assert!(needs_rewrite(Some("c:\\program files\\beebeeb\\beebeeb.exe"), exe));
        assert!(needs_rewrite(Some("\"C:\\Program Files\\Beebeeb\\Beebeeb.exe\""), exe));
        assert!(needs_rewrite(
            Some("C:\\Program Files\\Beebeeb\\Beebeeb.exe --minimized"),
            exe
        ));
        assert!(!needs_rewrite(Some(&command_line(exe)), exe));
        // Another install, or a development build run from elsewhere: not ours to change.
        assert!(!needs_rewrite(Some("\"D:\\Old\\Beebeeb.exe\" --autostart"), exe));
        assert!(!needs_rewrite(
            Some("C:\\Program Files\\Beebeeb\\Beebeeb.exe "),
            "C:\\dev\\target\\debug\\beebeeb-drive.exe"
        ));
        assert!(!needs_rewrite(Some("C:\\Program Files\\Beebeeb\\Beebeeb.exe.bak"), exe));
        // Off stays off: nothing to rewrite.
        assert!(!needs_rewrite(None, exe));
        assert!(!points_at("", exe));
        assert!(!points_at("anything", ""));
    }

    /// P2-3: a build at another path never overwrites or deletes the value.
    #[test]
    fn a_build_at_another_path_never_overwrites_or_deletes() {
        let installed = "C:\\Program Files\\Beebeeb\\beebeeb-desktop.exe";
        let dev = "C:\\dev\\target\\debug\\beebeeb-drive.exe";
        for existing in [
            command_line(installed),
            format!("{installed} "),
            format!("\"{installed}\""),
        ] {
            assert_eq!(plan(Wish::Enable, Some(&existing), dev), Err(OwnedByAnotherInstall));
            assert_eq!(plan(Wish::Disable, Some(&existing), dev), Ok(RunChange::Leave));
            assert_eq!(plan(Wish::Adopt, Some(&existing), dev), Ok(RunChange::Leave));
        }
    }

    #[test]
    fn this_programs_own_value_is_written_rewritten_and_deleted() {
        let exe = "C:\\Program Files\\Beebeeb\\beebeeb-desktop.exe";
        let ours = command_line(exe);
        assert_eq!(plan(Wish::Enable, None, exe), Ok(RunChange::Write(ours.clone())));
        assert_eq!(plan(Wish::Enable, Some(&ours), exe), Ok(RunChange::Leave));
        // The 0.8.x plugin's bare value for this program.
        let bare = format!("{exe} ");
        assert_eq!(plan(Wish::Enable, Some(&bare), exe), Ok(RunChange::Write(ours.clone())));
        assert_eq!(plan(Wish::Adopt, Some(&bare), exe), Ok(RunChange::Write(ours.clone())));
        assert_eq!(plan(Wish::Adopt, Some(&ours), exe), Ok(RunChange::Leave));
        // Off stays off at start.
        assert_eq!(plan(Wish::Adopt, None, exe), Ok(RunChange::Leave));
        assert_eq!(plan(Wish::Disable, Some(&ours), exe), Ok(RunChange::Delete));
        assert_eq!(plan(Wish::Disable, Some(&bare), exe), Ok(RunChange::Delete));
        assert_eq!(plan(Wish::Disable, None, exe), Ok(RunChange::Leave));
        // No known path of its own: touch nothing.
        for wish in [Wish::Enable, Wish::Disable, Wish::Adopt] {
            assert_eq!(plan(wish, Some(&ours), ""), Ok(RunChange::Leave));
        }
    }

    #[test]
    fn each_identity_has_its_own_value_name() {
        assert_eq!(value_name(AppIdentity::Real), "Beebeeb");
        assert_eq!(value_name(AppIdentity::Test), "Beebeeb Test");
    }
}
