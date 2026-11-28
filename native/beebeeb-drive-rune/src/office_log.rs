//! The Office viewer's helper, in the app's log: why it stopped or did not start. The host (`engine_host/office_host`)
//! calls only these, so the office page tool (which compiles the host in but links no `tracing`) gives its own.
//!
//! Never a file name, a path of the person's or a word of the document: a fixed reason, the session mode, the place in
//! the host's source that stopped it, and the helper's exit code (its standard error is not connected, by design).

/// The helper was stopped, or ended on its own. `exit`: its exit code when it had ended (hex: Windows status codes read
/// that way); `at`/`line`: the host's check that stopped it.
pub fn helper_stopped(why: &str, mode: &str, exit: Option<u32>, at: &str, line: u32) {
    let exit = exit.map_or_else(|| String::from("running"), |code| format!("{code:#010x}"));
    let at = at.rsplit(['/', '\\']).next().unwrap_or(at);
    tracing::warn!(why, mode, exit = %exit, at = %format_args!("{at}:{line}"), "office helper stopped");
}

/// The helper was not started: `reason` is a fixed word, `detail` the sandbox's fixed text when there is one.
pub fn helper_not_started(reason: &str, detail: &str) {
    tracing::warn!(reason, detail, "office helper not started");
}
