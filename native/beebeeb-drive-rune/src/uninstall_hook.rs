//! drive-complete-006b: `beebeeb-drive.exe --uninstall-sync-folder keep|remove`, run by the uninstaller
//! (`packaging/windows/hooks.nsh`, `BB_UNINSTALL_SYNC_FOLDER`) once the app has stopped and before its files go.
//! Tears down this build's sync folders (`beebeeb_engine::windows_cf::registration_teardown`) and exits: no window,
//! no tray, no engine. Exit code 0 when nothing was left, 1 when something was, 2 for a wrong argument.

/// The exit code when the arguments ask for the uninstall teardown; `None` for any other start.
pub fn run(mut args: impl Iterator<Item = String>) -> Option<i32> {
    if args.next().as_deref() != Some("--uninstall-sync-folder") {
        return None;
    }
    #[cfg(windows)]
    {
        use beebeeb_engine::windows_cf::registration_teardown::{SyncFolderChoice, uninstall};
        let Some(choice) = args.next().as_deref().and_then(SyncFolderChoice::parse) else {
            eprintln!("beebeeb-drive: --uninstall-sync-folder takes keep or remove");
            return Some(2);
        };
        let report = uninstall(crate::shell::identity::engine_identity(), choice);
        // Counts only: never a name.
        eprintln!("beebeeb-drive: uninstall {report:?}");
        Some(if report.failures == 0 { 0 } else { 1 })
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Some(2)
    }
}
