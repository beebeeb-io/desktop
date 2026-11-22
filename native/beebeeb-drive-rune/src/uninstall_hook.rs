//! drive-complete-006b: `beebeeb-drive.exe --uninstall-sync-folder keep|remove [forget-sign-in]`, run by the
//! uninstaller (`packaging/windows/hooks.nsh`, `BB_UNINSTALL_SYNC_FOLDER`) once the app has stopped and before its
//! files go. Tears down this build's sync folders (`beebeeb_engine::windows_cf::registration_teardown`) and exits: no
//! window, no tray, no engine. RC2-7: with `forget-sign-in` (passed only when "delete … the saved sign-in" is ticked)
//! it also deletes every Credential Manager entry of THIS build's identity (`beebeeb_engine::keychain::forget_sign_in`).
//! Exit code 0 when nothing was left, 1 when something was, 2 for a wrong argument.

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
        let forget = match args.next().as_deref() {
            None => false,
            Some("forget-sign-in") => true,
            Some(_) => {
                eprintln!("beebeeb-drive: --uninstall-sync-folder keep|remove [forget-sign-in]");
                return Some(2);
            }
        };
        let identity = crate::shell::identity::engine_identity();
        let report = uninstall(identity, choice);
        // Counts only: never a name.
        eprintln!("beebeeb-drive: uninstall {report:?}");
        let mut failures = report.failures;
        if forget {
            let forgot = beebeeb_engine::keychain::forget_sign_in(identity);
            eprintln!("beebeeb-drive: uninstall {forgot:?}");
            failures += forgot.failures;
        }
        Some(if failures == 0 { 0 } else { 1 })
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Some(2)
    }
}
