//! Opening and revealing things in Finder on a Mac (task 1885).
//!
//! The App Sandbox is why this exists. A path string carries no sandbox extension, and a child
//! process (`/usr/bin/open`) inherits the sandbox but not the extension on a URL this app was
//! handed, so LaunchServices refused the `~/Library/CloudStorage/Beebeeb-Beebeeb` folder with -54
//! while `spawn()` had already answered `Ok`. Nothing opened, and the button said it had.
//!
//! The fix is in `macos/FileProviderBridge.m`: the URL stays a URL (the security-scoped one macOS
//! returned for the item), `NSWorkspace` opens it in this process, and the bridge waits for
//! LaunchServices' answer. This file holds the part that does not need a Mac: how the bridge's
//! answer becomes the command's result, and the source pins that keep a `open` spawn from coming
//! back.

use crate::finder_setup::error::FpError;

/// "Open in Finder" when macOS reports no Finder location for the domain (it is not added, or has
/// been removed). A person sees the one sentence the frontend owns, never this text.
pub(crate) const NOTHING_TO_OPEN: &str = "Beebeeb isn’t in Finder right now, so there is nothing to open there.";

/// "Show in Finder" for one item, when macOS reports no location for it.
pub(crate) const NOTHING_TO_SHOW: &str = "Beebeeb isn’t in Finder right now, so there is nothing to show there.";

/// The bridge's answer to an open or a reveal, as the command's result: `Ok(true)` the location
/// was handed to LaunchServices (or Finder) and it answered; `Ok(false)` macOS reported no
/// location; `Err` the bridge or the OS failed. A failure is the command's `Err`, always: it is the
/// only thing that makes the frontend show a toast, and it carries the error's domain and code,
/// never the OS's own text (lead ruling T1-4: that text can name a folder).
pub(crate) fn finder_open_outcome(answer: Result<bool, FpError>, nothing_there: &str) -> Result<(), String> {
    match answer {
        Ok(true) => Ok(()),
        Ok(false) => Err(nothing_there.to_string()),
        Err(error) => Err(error.redacted()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{APP_DOMAIN, BRIDGE_DOMAIN, COCOA_DOMAIN, POSIX_DOMAIN, app_code, bridge_code};

    const OS_TEXT: &str =
        "The file \u{201c}/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb\u{201d} couldn\u{2019}t be opened.";

    #[test]
    fn an_opened_location_is_ok() {
        assert_eq!(finder_open_outcome(Ok(true), NOTHING_TO_OPEN), Ok(()));
    }

    #[test]
    fn no_location_is_an_error_with_the_given_sentence_never_a_success() {
        assert_eq!(
            finder_open_outcome(Ok(false), NOTHING_TO_OPEN),
            Err(NOTHING_TO_OPEN.to_string())
        );
        assert_eq!(
            finder_open_outcome(Ok(false), NOTHING_TO_SHOW),
            Err(NOTHING_TO_SHOW.to_string())
        );
        assert_ne!(NOTHING_TO_OPEN, NOTHING_TO_SHOW);
    }

    /// The error path of the task's verification: a bridge failure reaches the command's `Err`, as
    /// domain and code only. The OS's text (which here names a folder) must not.
    #[test]
    fn a_bridge_failure_reaches_the_commands_err_as_domain_and_code_only() {
        let launch_services = FpError::new(COCOA_DOMAIN, 256, OS_TEXT).with_underlying(POSIX_DOMAIN, 1);
        let timeout = FpError::new(BRIDGE_DOMAIN, bridge_code::OPEN_TIMEOUT, OS_TEXT);
        let gate_busy = FpError::app(app_code::OP_TIMEOUT, OS_TEXT);
        for error in [launch_services, timeout, gate_busy] {
            let expected = error.redacted();
            let result = finder_open_outcome(Err(error), NOTHING_TO_OPEN);
            assert_eq!(result, Err(expected.clone()), "the failure is the command's error");
            for leaked in ["/Users/sam", "CloudStorage", "couldn", "Beebeeb-Beebeeb"] {
                assert!(
                    !expected.contains(leaked),
                    "the error carried the OS's text ({leaked:?}): {expected}"
                );
            }
        }
        assert_eq!(
            finder_open_outcome(Err(FpError::app(app_code::OP_TIMEOUT, OS_TEXT)), NOTHING_TO_OPEN),
            Err(format!("{APP_DOMAIN} {}", app_code::OP_TIMEOUT))
        );
    }

    // ---- source pins: the sandbox fix cannot quietly come back ----

    fn read(relative: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            .replace("\r\n", "\n")
    }

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read the source dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// What starts `/usr/bin/open` or the bare `open` as a child. Built from pieces so this file
    /// does not contain them as one string (and the test module is dropped from the scan anyway).
    fn spawn_needles() -> [String; 3] {
        [
            ["Command::new(", "\"open\")"].concat(),
            ["Command::new(", "\"/usr/bin/open\")"].concat(),
            ["/usr/bin/", "open"].concat(),
        ]
    }

    /// 1885's pin: no code a Mac compiles starts `open` as a child process. The child inherited the
    /// sandbox and not the extension on the URL, so LaunchServices refused with -54 while `spawn()`
    /// had answered Ok. Windows (`explorer`) and Linux (`xdg-open`) keep their spawns, and the same
    /// scan proves it sees them, so it cannot pass by reading nothing.
    #[test]
    fn test_1885_no_macos_code_path_spawns_open() {
        use crate::finder_setup_command_tests::macos_compiled;
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(files.len() > 20, "the scan sees the source tree: {} files", files.len());

        let needles = spawn_needles();
        let (mut lines_on_a_mac, mut spawns_elsewhere) = (0, 0);
        for file in &files {
            let name = file.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            let text = std::fs::read_to_string(file)
                .expect("read a source file")
                .replace("\r\n", "\n");
            let on_a_mac = macos_compiled(&text);
            lines_on_a_mac += on_a_mac.lines().count();
            for (number, line) in on_a_mac.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for needle in &needles {
                    assert!(
                        !line.contains(needle.as_str()),
                        "{name}:{}: a Mac spawns `open` ({needle}): {line}",
                        number + 1
                    );
                }
            }
            let everywhere = without_tests(&text);
            spawns_elsewhere += everywhere.matches("Command::new(\"explorer\")").count();
            spawns_elsewhere += everywhere.matches("Command::new(\"xdg-open\")").count();
        }
        assert!(
            lines_on_a_mac > 10_000,
            "the scan read the macOS code: {lines_on_a_mac} lines"
        );
        assert!(
            spawns_elsewhere >= 4,
            "Windows and Linux keep their own spawns (and the scan sees them): {spawns_elsewhere}"
        );
    }

    fn without_tests(text: &str) -> String {
        crate::finder_setup_command_tests::without_items(text, &["#[cfg(test)]", "#[cfg(all(test"])
    }

    /// The Objective-C side has no way to start `open` either: the bridge opens through `NSWorkspace`
    /// in this process, never a task, a shell or a spawn.
    #[test]
    fn test_1885_the_bridge_starts_no_child_process() {
        let bridge = read("macos/FileProviderBridge.m");
        // Code only: a comment may name `/usr/bin/open` to say why it is gone.
        let code: String = bridge
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.lines().count() > 400,
            "the pin reads the bridge: {} lines",
            code.lines().count()
        );
        for forbidden in [
            "/usr/bin/open",
            "NSTask",
            "posix_spawn",
            "popen(",
            "system(",
            "execv",
            "fork(",
            "launchPath",
        ] {
            assert!(!code.contains(forbidden), "the bridge uses {forbidden}");
        }
    }

    /// The three commands that open something on a Mac reach `NSWorkspace` through the bridge, and the
    /// bridge's answer is the command's result (no `.spawn()`, no discarded outcome).
    #[test]
    fn test_1885_the_three_macos_open_actions_go_through_the_bridge_and_return_its_outcome() {
        use crate::finder_setup_command_tests::{body_between, macos_compiled};
        use crate::source_pin::squeeze;
        let on_a_mac = macos_compiled(&read("src/lib.rs"));

        let open = squeeze(body_between(&on_a_mac, "fn open_finder_location_blocking(", "\n}\n"));
        assert!(
            open.contains(&squeeze("return file_provider_open_location();")),
            "open_finder_location returns the bridge's outcome:\n{open}"
        );

        let reveal = squeeze(body_between(&on_a_mac, "fn open_in_finder_blocking(", "\n}\n"));
        assert!(
            reveal.ends_with(&squeeze("file_provider_reveal_item(&item_id)}")),
            "open_in_finder returns the bridge's outcome as the block's value, nothing after it:\n{reveal}"
        );

        let settings = squeeze(body_between(
            &on_a_mac,
            "fn open_login_items_and_extensions_settings_blocking(",
            "\n}\n",
        ));
        assert!(
            settings.contains(&squeeze("macos_workspace::open_url(")),
            "System Settings opens through NSWorkspace:\n{settings}"
        );
        assert!(
            !settings.contains(&squeeze("let _ = macos_workspace::open_url(")),
            "a failed open of System Settings is not discarded:\n{settings}"
        );

        for (wrapper, call) in [
            (
                "fn file_provider_open_location(",
                "finder_open::finder_open_outcome(finder_setup::macos_ports::open_location(),",
            ),
            (
                "fn file_provider_reveal_item(",
                "finder_open::finder_open_outcome(finder_setup::macos_ports::reveal_item(item_id),",
            ),
        ] {
            let production = without_tests(&read("src/lib.rs"));
            let body = squeeze(body_between(&production, wrapper, "\n}\n"));
            assert!(
                body.contains(&squeeze(call)),
                "{wrapper} turns the gated bridge call into the command's result with finder_open_outcome:\n{body}"
            );
        }
    }

    /// The Objective-C open waits for LaunchServices' answer and holds the URL's scope across the
    /// call. Pinned in the text because the failure it prevents (a success reported before the OS
    /// answered, and a scope dropped before it was used) shows only on a signed, sandboxed build.
    #[test]
    fn test_1885_the_bridge_opens_the_scoped_url_and_waits_for_the_answer() {
        let bridge = read("macos/FileProviderBridge.m");
        let open_url = {
            let from = bridge
                .find("static int BeebeebOpenURLAndWait(")
                .expect("the bridge's open helper");
            let to = from + bridge[from..].find("\n}\n").expect("the helper ends");
            &bridge[from..to]
        };
        let scope = open_url
            .find("startAccessingSecurityScopedResource")
            .expect("the helper takes the URL's scope");
        let open = open_url
            .find("openURL:url configuration:")
            .expect("it opens through NSWorkspace with a configuration");
        let wait = open_url
            .find("dispatch_semaphore_wait")
            .expect("it waits for the completion handler");
        assert!(scope < open && open < wait, "scope, then open, then wait:\n{open_url}");
        assert!(
            open_url.contains("promptsUserIfNeeded = NO"),
            "no dialog can hold the answer back"
        );
        assert!(
            open_url.contains("stopAccessingSecurityScopedResource"),
            "the scope is released in the completion handler"
        );
        assert!(
            open_url.contains("found_error"),
            "the completion handler's error becomes the bridge's error"
        );

        let location = {
            let from = bridge
                .find("int beebeeb_fp_open_location(")
                .expect("the open-location entry point");
            let to = from + bridge[from..].find("\n}\n").expect("it ends");
            &bridge[from..to]
        };
        assert!(
            location.contains("NSFileProviderRootContainerItemIdentifier")
                && location.contains("BeebeebOpenURLAndWait("),
            "the location it opens is the URL macOS returned for the domain's root:\n{location}"
        );
        assert!(
            !location.contains("fileURLWithPath") && !location.contains(".path"),
            "the URL is never turned into a path string (a path carries no scope):\n{location}"
        );
    }
}
