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
use crate::state_db::Namespace;

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

/// The sentence a failed "Open in Finder" says on a Mac: the same words as the button's toast
/// (`FINDER_OPEN_FAILED` in `src/finderSetupCopy.ts`; a test keeps the two equal). The error behind it is a bare domain
/// and code and is never shown.
pub(crate) const OPEN_FAILED_SENTENCE: &str = "Beebeeb couldn’t open its Finder location.";

/// The File Provider-style id of the "shared with me" pseudo folder (`ipc_socket::NAMESPACE_SHARED_WITH_ME`). It is not a
/// row in the state database, and it is never shown in Finder.
pub(crate) const SHARED_WITH_ME_PSEUDO_FOLDER: &str = "namespace:shared_with_me";

/// "Show in Finder" for a row that is shared with the person (task 1885 fix round 1, I3). Shared content is webapp-only
/// by ruling 1701: it is not in Finder. A Mac never asks the File Provider for its location: macOS may fail the lookup
/// (a toast for a result that was offered) or place it under "Beebeeb", the surfacing the change feed filters out.
pub(crate) const SHARED_NOT_IN_FINDER: &str = "Files shared with you are not in Finder; they open in the web app.";

/// "Show in Finder" for an id this Mac's state database does not know. Nothing to resolve.
pub(crate) const ITEM_NOT_KNOWN: &str = "That item is not known on this Mac yet.";

/// Whether a Mac may ask for an item's Finder location (task 1885 fix round 1, I3). `namespace_of` reads the item's
/// namespace from the state database (`Ok(None)`: no such row). Anything that is not plainly one of the person's own
/// rows is refused, and a lookup that fails is refused too, so the File Provider is asked about nothing it should not
/// be. The id is checked here, before any bridge call.
pub(crate) fn finder_may_show(
    item_id: &str,
    namespace_of: impl FnOnce(&str) -> Result<Option<Namespace>, String>,
) -> Result<(), String> {
    if item_id == SHARED_WITH_ME_PSEUDO_FOLDER {
        return Err(SHARED_NOT_IN_FINDER.to_string());
    }
    match namespace_of(item_id)? {
        Some(Namespace::SharedWithMe) => Err(SHARED_NOT_IN_FINDER.to_string()),
        Some(_) => Ok(()),
        None => Err(ITEM_NOT_KNOWN.to_string()),
    }
}

/// The title of the alert the menu's "Open in Finder" raises when it fails.
pub(crate) const MENU_OPEN_FAILED_TITLE: &str = "Open in Finder";

/// What the menu's "Open in Finder" shows the person when its work ended in `result` (task 1885 fix round 1, I2): on a
/// Mac an `Err` is an alert with the title and the one sentence; success shows nothing, and so does Windows or Linux,
/// whose menu keeps logging its failure as before. The menu has no window to put a toast in (it works with every window
/// closed), so the alert is native, like the Sign-out menu action's.
pub(crate) fn menu_open_failure_alert(
    result: &Result<(), String>,
    on_a_mac: bool,
) -> Option<(&'static str, &'static str)> {
    match result {
        Err(_) if on_a_mac => Some((MENU_OPEN_FAILED_TITLE, OPEN_FAILED_SENTENCE)),
        _ => None,
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

    /// Fix round 1 (I3): a Mac never asks for the Finder location of a shared-with-me row, whatever the File Provider
    /// would say; an unknown id and a failed lookup are refused as well.
    #[test]
    fn test_1885_a_shared_with_me_item_is_never_asked_of_the_file_provider() {
        let asked = std::cell::Cell::new(0);
        let lookup = |namespace: Option<Namespace>| {
            let asked = &asked;
            move |_: &str| {
                asked.set(asked.get() + 1);
                Ok(namespace)
            }
        };
        assert_eq!(
            finder_may_show("a-shared-file", lookup(Some(Namespace::SharedWithMe))),
            Err(SHARED_NOT_IN_FINDER.to_string())
        );
        assert_eq!(
            finder_may_show(SHARED_WITH_ME_PSEUDO_FOLDER, lookup(Some(Namespace::MyFiles))),
            Err(SHARED_NOT_IN_FINDER.to_string()),
            "the pseudo folder is refused by its id, without a lookup"
        );
        assert_eq!(asked.get(), 1, "only the first call looked the row up");
        for own in [Namespace::MyFiles, Namespace::Offline, Namespace::Conflicts] {
            assert_eq!(finder_may_show("mine", lookup(Some(own))), Ok(()));
        }
        assert_eq!(
            finder_may_show("never-seen", lookup(None)),
            Err(ITEM_NOT_KNOWN.to_string())
        );
        assert_eq!(
            finder_may_show("x", |_| Err("database is locked".to_string())),
            Err("database is locked".to_string()),
            "a lookup that fails is a refusal"
        );
    }

    /// Fix round 1 (I3), in the command: on a Mac `open_in_finder` checks the namespace after the identifier shape and
    /// BEFORE the File Provider is asked anything, and the check is `?`-propagated, never discarded.
    #[test]
    fn test_1885_open_in_finder_checks_the_namespace_before_it_asks_the_file_provider() {
        use crate::finder_setup_command_tests::{body_between, macos_compiled};
        use crate::source_pin::squeeze;
        let on_a_mac = macos_compiled(&read("src/lib.rs"));
        let body = squeeze(body_between(&on_a_mac, "fn open_in_finder_blocking(", "\n}\n"));
        let shape = body
            .find("macos_validate_hydrate_item_identifier(&item_id)")
            .expect("the shape check");
        let guard = body
            .find(&squeeze("finder_open::finder_may_show(&item_id,"))
            .expect("the namespace check");
        let reveal = body.find("file_provider_reveal_item(&item_id)").expect("the reveal");
        assert!(
            shape < guard && guard < reveal,
            "shape, then namespace, then the File Provider:\n{body}"
        );
        let guard_statement = &body[guard..reveal];
        assert!(
            guard_statement.contains(")?;"),
            "the refusal is returned, not dropped:\n{guard_statement}"
        );
        assert!(
            guard_statement.contains("get_file_contract_state(id)"),
            "the namespace comes from the state database's contract row:\n{guard_statement}"
        );
    }

    /// Fix round 1 (m6): a test that asks the real `NSWorkspace` needs a logged-in GUI session (over SSH or on a
    /// headless Mac LaunchServices may not answer, and the bridge's 5 s limit turns that into a failing test). Every such
    /// test in `macos_workspace.rs` carries `#[ignore = "<why>"]` and runs with `--ignored` on a Mac someone is sitting at.
    #[test]
    fn test_1885_every_test_that_asks_the_real_nsworkspace_is_ignored_with_a_reason() {
        let source = read("src/macos_workspace.rs");
        let tests = &source[source.find("#[cfg(test)]\nmod tests {").expect("the tests")..];
        let mut asking = 0;
        for chunk in tests.split("    #[test]\n").skip(1) {
            let name = chunk.lines().find(|line| line.contains("fn ")).unwrap_or("?").trim();
            if chunk.contains("open_scoped(") || chunk.contains("beebeeb-no-such-scheme") {
                asking += 1;
                assert!(
                    chunk.trim_start().starts_with("#[ignore = \"") && chunk.contains("GUI session"),
                    "{name} asks the real NSWorkspace and is not ignored with a reason"
                );
            }
        }
        assert_eq!(asking, 2, "the two tests that open through the real NSWorkspace");
    }

    /// Fix round 1 (m4): the two fallbacks that open the Finder location after an upload or a new folder run on the
    /// blocking pool and are not waited for. The open can now wait up to 3 s for the gate and 7 s for LaunchServices; the
    /// new-folder one ran inline on the MAIN thread (the menu handler and a plain `#[tauri::command] fn`) and the upload
    /// one on a runtime worker. Their result was already discarded; a failure is logged.
    #[test]
    fn test_1885_the_fallback_opens_never_block_the_caller() {
        use crate::finder_setup_command_tests::{body_between, without_items};
        use crate::source_pin::squeeze;
        let production = without_items(&read("src/lib.rs"), &["#[cfg(test)]", "#[cfg(all(test"]);
        for function in [
            "fn upload_files_to_sync_root_impl(",
            "fn create_folder_in_sync_root_impl(",
        ] {
            let body = body_between(&production, function, "\n}\n");
            assert!(
                !body.contains("open_finder_location_blocking("),
                "{function} opens the location inline, on its caller's thread:\n{body}"
            );
            assert!(
                body.contains("open_finder_location_in_background("),
                "{function} hands the open to the background helper:\n{body}"
            );
        }
        let helper = squeeze(body_between(
            &production,
            "fn open_finder_location_in_background(",
            "\n}\n",
        ));
        assert!(
            helper.contains(&squeeze("tauri::async_runtime::spawn_blocking(")),
            "the helper uses the blocking pool: {helper}"
        );
        assert!(
            helper.contains(&squeeze("open_finder_location_blocking(Some(sync_root))")),
            "and does the same open: {helper}"
        );
        assert!(
            helper.contains("tracing::warn!"),
            "a failure is logged, not dropped: {helper}"
        );
        assert_eq!(
            production.matches("open_finder_location_blocking(").count(),
            // the definition, the command's pool closure, the menu's `open_current_finder_location`, the helper's own call
            4,
            "every other caller goes through the pool or the helper"
        );
    }

    /// Fix round 1 (I2): a failed menu open is shown, on a Mac, with the button's sentence; a success and the other
    /// platforms show nothing.
    #[test]
    fn test_1885_a_failed_menu_open_on_a_mac_is_an_alert_with_the_one_sentence() {
        assert_eq!(
            menu_open_failure_alert(&Err("io.beebeeb.bridge 6".to_string()), true),
            Some((MENU_OPEN_FAILED_TITLE, OPEN_FAILED_SENTENCE))
        );
        assert_eq!(
            menu_open_failure_alert(&Err(NOTHING_TO_OPEN.to_string()), true),
            Some((MENU_OPEN_FAILED_TITLE, OPEN_FAILED_SENTENCE)),
            "also when macOS reports no location"
        );
        assert_eq!(menu_open_failure_alert(&Ok(()), true), None, "a success says nothing");
        assert_eq!(
            menu_open_failure_alert(&Err("Not configured on this PC yet".to_string()), false),
            None,
            "Windows and Linux keep logging"
        );
        let shown = format!("{OPEN_FAILED_SENTENCE}{MENU_OPEN_FAILED_TITLE}");
        assert!(
            !shown.contains("bridge") && !shown.contains("NS") && !shown.contains('/'),
            "the alert never carries an error code or a path: {shown}"
        );
    }

    /// Fix round 1 (I2): the Rust sentence is the frontend's `FINDER_OPEN_FAILED`, character for character.
    #[test]
    fn test_1885_the_menu_alert_says_what_the_buttons_toast_says() {
        let copy = read("../src/finderSetupCopy.ts");
        let line = copy
            .lines()
            .find(|line| line.starts_with("export const FINDER_OPEN_FAILED"))
            .expect("the frontend's constant");
        let sentence = line.split('\'').nth(1).expect("a quoted sentence");
        assert_eq!(sentence, OPEN_FAILED_SENTENCE);
    }

    /// Fix round 1 (I2), in the menu: the OpenFolder arm runs the open on the blocking pool, then hands the result to
    /// `show_menu_open_failure` before returning it to the logger, and that function raises the native alert for the
    /// alert `menu_open_failure_alert` describes.
    #[test]
    fn test_1885_the_menus_open_folder_shows_a_failure_before_it_logs_it() {
        use crate::finder_setup_command_tests::{body_between, without_items};
        use crate::source_pin::squeeze;
        let production = without_items(&read("src/lib.rs"), &["#[cfg(test)]", "#[cfg(all(test"]);
        let arm = squeeze(body_between(
            &production,
            "DesktopMenuAction::OpenFolder =>",
            "DesktopMenuAction::UploadFiles",
        ));
        let open = arm
            .find(&squeeze(
                "on_the_blocking_pool(OPEN_FOLDER_FAILED, open_current_finder_location).await",
            ))
            .expect("the open runs on the blocking pool, and the arm waits for it");
        let show = arm
            .find(&squeeze("show_menu_open_failure(&app, &result);"))
            .expect("the arm shows a failure");
        let give_back = arm.rfind("result").expect("the arm returns the result");
        assert!(open < show, "the failure is shown after the open ended: {arm}");
        assert!(
            show < give_back,
            "the result goes on to the logger after it is shown: {arm}"
        );
        let shower = squeeze(body_between(&production, "fn show_menu_open_failure(", "\n}\n"));
        assert!(
            shower.contains(&squeeze(
                "finder_open::menu_open_failure_alert(result, cfg!(target_os = \"macos\"))"
            )) && shower.contains(".dialog()")
                && shower.contains(&squeeze("MessageDialogKind::Error")),
            "the alert is native, an error dialog, for exactly what the pure function says: {shower}"
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

    /// The text of one C function of the bridge, from `start` to the first line that is only `}`.
    fn c_function<'a>(bridge: &'a str, start: &str) -> &'a str {
        let from = bridge.find(start).unwrap_or_else(|| panic!("the bridge has {start}"));
        let to = from + bridge[from..].find("\n}\n").unwrap_or_else(|| panic!("{start} ends"));
        &bridge[from..to]
    }

    /// The Objective-C open waits for LaunchServices' answer and holds the URL's scope across the
    /// call. Pinned in the text because the failure it prevents (a success reported before the OS
    /// answered, and a scope dropped before it was used) shows only on a signed, sandboxed build.
    #[test]
    fn test_1885_the_bridge_opens_the_scoped_url_and_waits_for_the_answer() {
        let bridge = read("macos/FileProviderBridge.m");
        let open_url = c_function(&bridge, "static int BeebeebOpenURLAndWait(");
        let scope = open_url
            .find("[[BeebeebScope alloc] initWithURL:url]")
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
            open_url.contains("[scope stop]"),
            "the scope is released in the completion handler"
        );
        assert!(
            open_url.contains("found_error"),
            "the completion handler's error becomes the bridge's error"
        );
    }

    /// Fix round 1 (m2): the scope is released IN the completion handler, which is the last thing that needs it, not
    /// somewhere after `openURL:` (before LaunchServices has read the URL). The stop sits between the handler's
    /// parameter list and the semaphore signal that wakes the waiting call.
    #[test]
    fn test_1885_the_scope_is_stopped_inside_the_completion_handler() {
        let bridge = read("macos/FileProviderBridge.m");
        let open_url = c_function(&bridge, "static int BeebeebOpenURLAndWait(");
        let handler = open_url
            .find("completionHandler:^(NSRunningApplication *application, NSError *error) {")
            .expect("the open's completion handler");
        let handler = &open_url[handler..];
        let handler = &handler[..handler.find("}];").expect("the handler ends")];
        let stop = handler.find("[scope stop];").expect("the handler stops the scope");
        let signal = handler
            .find("dispatch_semaphore_signal(semaphore)")
            .expect("the handler wakes the waiter");
        assert!(
            stop < signal,
            "the scope is stopped before the waiter is woken:\n{handler}"
        );
        let stops: Vec<usize> = open_url.match_indices("[scope stop];").map(|(at, _)| at).collect();
        let timer = open_url.find("dispatch_after(").expect("the safety timer");
        assert_eq!(
            stops.len(),
            2,
            "two stops, the handler's and the safety timer's, and none between the open call and the wait:\n{open_url}"
        );
        assert!(stops[1] > timer, "the second stop is the safety timer's:\n{open_url}");
    }

    /// Fix round 1 (m3): a handler that never comes still releases the scope, and the scope is released at most once.
    /// `NSWorkspace` has no way to cancel an open, so after the wait times out the answer may still arrive; a safety
    /// timer stops the scope if it does not, and the scope object stops once whichever of the two comes first. (A
    /// late SUCCESS still opens the window after the caller was told it failed: written in the helper's comment.)
    #[test]
    fn test_1885_a_handler_that_never_comes_still_releases_the_scope_exactly_once() {
        let bridge = read("macos/FileProviderBridge.m");
        let open_url = c_function(&bridge, "static int BeebeebOpenURLAndWait(");
        let timer = open_url.find("dispatch_after(").expect("the open has a safety timer");
        let timer = &open_url[timer..];
        let timer = &timer[..timer.find("});").expect("the timer block ends")];
        assert!(
            timer.contains("BeebeebScopeSafetySeconds") && timer.contains("[scope stop];"),
            "the safety timer stops the scope after the safety interval:\n{timer}"
        );
        assert!(
            bridge.contains("static const int64_t BeebeebScopeSafetySeconds = 60;"),
            "the safety interval is a named constant, 60 s"
        );
        let scope_class = {
            let from = bridge.find("@implementation BeebeebScope").expect("the scope class");
            &bridge[from..from + bridge[from..].find("@end").expect("it ends")]
        };
        let stop = scope_class.find("- (void)stop {").expect("the stop method");
        let stop = &scope_class[stop..];
        let guard = stop.find("if (_active) {").expect("a stop checks it is still active");
        let clear = stop.find("_active = NO;").expect("it marks the scope stopped");
        let release = stop
            .find("stopAccessingSecurityScopedResource")
            .expect("it releases the scope");
        assert!(
            guard < clear && clear < release,
            "stop: check, mark, then release (once):\n{stop}"
        );
        assert!(
            stop.contains("@synchronized(self)"),
            "the check and the mark are one step:\n{stop}"
        );
        let comment = &bridge[bridge
            .find("static int BeebeebOpenURLAndWait(")
            .unwrap()
            .saturating_sub(1200)..];
        assert!(
            comment.contains("after the caller has already been told it failed"),
            "the late-success edge is written down where the timeout is"
        );
    }

    /// Fix round 1 (I1), in the bridge: the RESOLVE (a File Provider call, run under Rust's gate) hands back a retained
    /// handle and opens nothing; the OPEN and the REVEAL (run after the gate is released) take the handle first, make
    /// no File Provider call, and never turn the URL into a path string (a path carries no scope).
    #[test]
    fn test_1885_the_resolve_opens_nothing_and_the_open_asks_the_file_provider_nothing() {
        let bridge = read("macos/FileProviderBridge.m");
        for resolve in ["int beebeeb_fp_resolve_location(", "int beebeeb_fp_resolve_item("] {
            let body = c_function(&bridge, resolve);
            assert!(
                body.contains("BeebeebRetainHandle(url)"),
                "{resolve} returns a retained handle:\n{body}"
            );
            for forbidden in [
                "openURL:",
                "activateFileViewerSelectingURLs",
                "BeebeebOpenURLAndWait",
                "fileURLWithPath",
                ".path",
            ] {
                assert!(!body.contains(forbidden), "{resolve} uses {forbidden}:\n{body}");
            }
        }
        assert!(
            c_function(&bridge, "int beebeeb_fp_resolve_location(")
                .contains("NSFileProviderRootContainerItemIdentifier"),
            "the location it resolves is the domain's root"
        );
        for consumer in [
            "int beebeeb_open_scoped_url(",
            "int beebeeb_reveal_scoped_url(",
            "void beebeeb_release_url_handle(",
        ] {
            let body = c_function(&bridge, consumer);
            let take = body
                .find("BeebeebTakeHandle(handle)")
                .unwrap_or_else(|| panic!("{consumer} takes the handle"));
            let first_use = [
                "BeebeebOpenURLAndWait",
                "BeebeebScope alloc",
                "lstat",
                "activateFileViewerSelectingURLs",
            ]
            .iter()
            .filter_map(|use_| body.find(use_))
            .min()
            .unwrap_or(usize::MAX);
            assert!(
                take < first_use,
                "{consumer} takes (and so owns) the handle before it uses it:\n{body}"
            );
            for forbidden in [
                "managerForDomain",
                "getUserVisibleURLForItemIdentifier",
                "BeebeebUserVisibleURL",
                "BeebeebDomain()",
                "fileURLWithPath",
                ".path",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{consumer} uses {forbidden}, which is a File Provider call or a path:\n{body}"
                );
            }
        }
    }
}
