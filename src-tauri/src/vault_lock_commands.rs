//! `vault_lock_commands` items split out of `lib.rs`.

use super::*;

/// `clear_session`'s `keepUnsent: { askId }` (spec 2026-10-10 §5.1, the contract). Accepted and ignored off macOS (W5).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeepUnsentArg {
    pub(super) ask_id: String,
}

/// Task 2191 (Decision (lead, 2026-10-26)): what the person chose in the Windows sign-out
/// confirmation for files of unknown content that hold bytes on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum UnknownFilesArg {
    /// Keep a copy of each in a folder beside the sync folder (the default).
    Keep,
    /// Delete them with the rest.
    Delete,
}

/// Task 2191: what the Windows sign-out confirmation shows: how many files of unknown content
/// hold bytes on this PC, and the folder "Keep copies" would use.
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct UnknownFilesAtSignOut {
    pub(super) count: usize,
    pub(super) folder: Option<String>,
}

/// Task 2191: the folder a Windows sign-out keeps unknown bytes in: beside the sync folder, or in
/// the home folder for a sync folder at a drive root. `None`: there is no place outside it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn kept_unknown_files_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    unknown_check::kept_files_dir(
        root,
        &chrono::Local::now().format("%Y-%m-%d %H%M").to_string(),
        home.as_deref(),
    )
}

/// Task 2191 (D11): the files the Windows sign-out would keep or delete as unknown: by the same
/// rule as the purge (`unknown_check::unknown_at_signout`), holding bytes on this PC.
#[cfg(target_os = "windows")]
pub(super) fn count_unknown_files_at_sign_out() -> Result<UnknownFilesAtSignOut, String> {
    use unknown_check::{ExpectedAt, OnDisk, PlatformView};
    let root = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
    let (Some(db), Some(root)) = (state_db_from_app_local_state_dir()?, root) else {
        return Ok(UnknownFilesAtSignOut { count: 0, folder: None });
    };
    let mut places: Vec<(String, ExpectedAt)> = db
        .list_unknown_orphans()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(path, _, _)| (path, ExpectedAt::OrphanCandidate))
        .collect();
    places.extend(
        db.list_guarded()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|row| (row.path, ExpectedAt::Guarded(row.file_id))),
    );
    let mut count = 0;
    for (rel, expected) in places {
        let Some(path) = engine_bridge::EngineBridge::placeholder_path_under(&root, &rel) else {
            continue;
        };
        let (placeholder, identity, holds_bytes) = match windows_cf::placeholders::platform_view(&path) {
            PlatformView::Missing | PlatformView::Unreadable => continue,
            PlatformView::PlainFile => (false, None, true),
            PlatformView::Placeholder { on_disk, identity, .. } => (true, identity, on_disk != OnDisk::Nothing),
        };
        let owner = match identity.as_deref() {
            Some(id) => db
                .get_file(id)
                .map_err(|e| e.to_string())?
                .map(|row| (row.path, db.has_agreed_content(id).unwrap_or(true))),
            None => None,
        };
        let unknown = unknown_check::unknown_at_signout(
            &expected,
            &rel,
            placeholder,
            identity.as_deref(),
            owner.as_ref().map(|(path, guarded)| (path.as_str(), *guarded)),
        );
        if unknown && holds_bytes {
            count += 1;
        }
    }
    let folder = kept_unknown_files_dir(&root)
        .filter(|_| count > 0)
        .map(|dir| dir.to_string_lossy().into_owned());
    Ok(UnknownFilesAtSignOut { count, folder })
}

/// Task 2191: how many files of unknown content hold bytes on this PC (Windows), for the
/// sign-out confirmation. Elsewhere: none.
#[tauri::command]
pub(super) async fn sign_out_unknown_files() -> Result<UnknownFilesAtSignOut, String> {
    #[cfg(target_os = "windows")]
    {
        // Self-review: one platform read per file; off the async workers.
        tokio::task::spawn_blocking(count_unknown_files_at_sign_out)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(UnknownFilesAtSignOut { count: 0, folder: None })
    }
}

/// Sign out through the shared native-menu/WebView teardown. Windows returns
/// an error while work, plaintext cleanup or root unregistration is incomplete;
/// the UI must retain the account and display that error for recovery/retry.
/// `forget_email` (`forgetEmail` from the frontend): this sign-out is an account switch, so the sign-in prefill is
/// forgotten too (spec §5.6). Absent or false: the prefill is kept, as before.
/// `Ok` means the sign-out happened; its `warning` says what of it could not be confirmed (FB-24).
///
/// Task 1882: when the sign-out kept Finder files that had not reached the
/// server, the app's alert names the folder. Every frontend sign-out (Settings,
/// the compact Account page, "Sign in again", the account switch) ends here; the menu's sign-out
/// raises the same alert in its own handler.
///
/// Spec 2026-10-10 §5.1 "A late result" (macOS): Tauri passes the calling window. A result that needs showing stays
/// pending until that window acknowledges it (`sign_out_result_ack`); if the window closes, is destroyed or reloads
/// first, Rust shows it in the menu's dialog, and so it does when the call returns after such a close.
#[tauri::command]
pub(super) async fn clear_session(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    forget_email: Option<bool>,
    keep_unsent: Option<KeepUnsentArg>,
    unknown_files: Option<UnknownFilesArg>,
    kept_folder: Option<String>,
) -> Result<SessionActionOutcome, String> {
    #[cfg(target_os = "macos")]
    state.sign_out.late_results.begin(window.label());
    let result = clear_session_impl(
        &state,
        forget_email.unwrap_or(false),
        keep_unsent.as_ref().map(|keep| keep.ask_id.as_str()),
        // Task 2191: without an answer (the menu, an older window) the copies are kept.
        unknown_files.unwrap_or(UnknownFilesArg::Keep),
        // Review-157 F4: the folder the confirmation named, so the copies go where it said.
        kept_folder.map(std::path::PathBuf::from),
    )
    .await;
    // Review I1: the alert comes first, also when the sign-out failed after the removal.
    // Review I2: and the folder is saved for the Settings › Sync row.
    surface_sign_out_folders(&app, &result);
    #[cfg(target_os = "macos")]
    if let Some(shown) = state
        .sign_out
        .late_results
        .complete(window.label(), late_result_of(&result))
    {
        show_late_result(&app, shown);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &window;
    result
        .map(|done| SessionActionOutcome { warning: done.warning })
        .map_err(|failure| failure.message)
}

/// Spec 2026-10-10 §5.1 "A late result": what needs showing. Not `unsent_changes` (nothing changed, and a hidden window
/// cannot show the ask), and not a completed sign-out without a warning. Read on macOS and in tests only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn late_result_of(result: &Result<SignedOut, SignOutFailure>) -> Option<sign_out::flow_window::Shown> {
    use sign_out::flow_window::Shown;
    match result {
        Err(failure)
            if failure
                .message
                .starts_with(&format!("{}: ", sign_out::copy::UNSENT_CHANGES_CODE)) =>
        {
            None
        }
        Err(failure) => Some(Shown::Failure(failure.message.clone())),
        Ok(SignedOut {
            warning: Some(warning), ..
        }) => Some(Shown::Warning(warning.sentence().to_string())),
        Ok(_) => None,
    }
}

/// The menu's own dialogs (its sign-out's "Sign-out paused" and "Signed out"), in the callback form, which does not
/// wait for the person, so a window-event handler can raise one. With no parent window macOS draws it in its own
/// UserNotificationCenter process, so it shows while Settings is hidden.
#[cfg(target_os = "macos")]
pub(super) fn show_late_result(app: &tauri::AppHandle, shown: sign_out::flow_window::Shown) {
    use sign_out::flow_window::Shown;
    let (title, message, kind) = match shown {
        Shown::Failure(message) => (
            "Sign-out paused",
            message,
            tauri_plugin_dialog::MessageDialogKind::Error,
        ),
        Shown::Warning(sentence) => ("Signed out", sentence, tauri_plugin_dialog::MessageDialogKind::Info),
    };
    app.dialog().message(message).title(title).kind(kind).show(|_| {});
}

/// What a close of `label` did (U34): the four steps of §5.1's hook, on the flow state alone. Read on macOS and in
/// tests only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) struct FlowClosed {
    // The hook acts on `show` alone; these two say what the close ended, for the tests (U34).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) phase_closed: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) snapshot_dropped: bool,
    pub(super) show: Option<sign_out::flow_window::Shown>,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn flow_window_closed(sign_out: &sign_out::SignOutState, label: &str) -> FlowClosed {
    let phase_closed = match sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        Some(phase) if phase.owner == label => {
            phase.closed.store(true, Ordering::SeqCst);
            true
        }
        _ => false,
    };
    FlowClosed {
        phase_closed,
        snapshot_dropped: sign_out.snapshots.drop_owned_by(label),
        show: sign_out.late_results.window_closed(label),
    }
}

/// Spec 2026-10-10 §5.1 "A close ends the flow": every `CloseRequested` and `Destroyed`, and every page load `Started`,
/// of any window.
#[cfg(target_os = "macos")]
pub(super) fn sign_out_flow_window_closed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let closed = flow_window_closed(&state.sign_out, label);
    let _ = app.emit_to(label, "sign-out-flow-closed", ());
    if let Some(shown) = closed.show {
        show_late_result(app, shown);
    }
}

/// The webview displayed its `clear_session`'s outcome (spec 2026-10-10 F12): remove that window's pending result.
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) fn sign_out_result_ack(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    state.sign_out.late_results.acknowledge(window.label());
    Ok(())
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): nothing is ever pending there.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) fn sign_out_result_ack(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    let _ = (window, state);
    Ok(())
}

/// §5.1 "A flow request survives a window that has not loaded yet": one pending request. Read on macOS and in tests
/// only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn take_sign_out_flow_request(sign_out: &sign_out::SignOutState) -> bool {
    sign_out.flow_request.swap(false, Ordering::SeqCst)
}

/// Entry points 2 and 5: store the request, open the Beebeeb window on Settings › Account (task 1683 slice 1,
/// two-surface spec §8.1), and tell it (it also consumes on mount).
#[cfg(target_os = "macos")]
pub(super) fn open_sign_out_flow(app: &tauri::AppHandle) {
    app.state::<AppState>()
        .sign_out
        .flow_request
        .store(true, Ordering::SeqCst);
    show_main_window(app, MainNav::settings(Some("account")));
    let _ = app.emit_to("main", "menu:sign-out-flow", ());
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) fn request_sign_out_flow(app: tauri::AppHandle) -> Result<(), String> {
    open_sign_out_flow(&app);
    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) fn consume_sign_out_flow_request(state: State<'_, AppState>) -> bool {
    take_sign_out_flow_request(&state.sign_out)
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): there is no flow to open there.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) fn request_sign_out_flow(app: tauri::AppHandle) -> Result<(), String> {
    let _ = app;
    Err("This works on a Mac only.".into())
}

/// Windows and Linux: no request is ever pending.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) fn consume_sign_out_flow_request(state: State<'_, AppState>) -> bool {
    let _ = state;
    false
}

/// Task 1882 (spec `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md` §5): the app's
/// own alert after a removal that kept files — the sentence, a blank line, then the folder. The app-start sweep and a
/// reconciler removal nobody waits for (an owed removal, the account binding's Repair, or one that finished after the
/// reconciler's time limit) raise it through `surface_kept_folder`, which first saves the folder for the Settings ›
/// Sync row. A sign-out puts the same body in its own alert, after its kept folder's (`surface_sign_out_folders`, task
/// 1887). `install_finder_location`'s rollback calls it too, but only Windows and Linux have that path, and nothing is
/// kept there. Logs never carry the folder. Nothing kept → no alert.
///
/// Device K-F1 (round 2): with no parent window, `tauri-plugin-dialog` 2.7 hands this to `rfd`
/// 0.16, which calls `CFUserNotificationDisplayAlert` on a background thread. macOS draws that
/// alert in its own UserNotificationCenter process, so it is never a Beebeeb window. The two
/// debug lines let a device check prove it was raised and closed, without the path.
pub(super) fn show_preserved_files_alert(app: &tauri::AppHandle, preserved_location: Option<&str>) {
    let Some((title, message)) = finder_removal::kept_folder_alert(preserved_location) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Info)
        .show(|_| {
            tracing::debug!("kept-folder alert closed");
        });
    tracing::debug!("kept-folder alert shown");
}

/// Review I2 (lead ruling, round 2): hands a kept folder to the person on the paths that have no
/// config of their own (the app-start sweep, a reconciler removal): saved for the Settings › Sync row
/// first, with the windows told (rebase re-review I2: [`save_kept_folder_for_row`]), then the alert. Nothing kept →
/// nothing. A sign-out surfaces its folders through [`surface_sign_out_folders`].
pub(super) fn surface_kept_folder(app: &tauri::AppHandle, preserved_location: Option<&str>) {
    surface_kept_folder_with(
        preserved_location,
        |location| save_kept_folder_for_row(app, location),
        |location| show_preserved_files_alert(app, Some(location)),
    );
}

/// [`surface_kept_folder`] with its steps passed in, so a test can run the real order on its own config file and
/// record the event and the alert: `save_for_row` (the save and the telling) first, then the alert. Nothing kept →
/// none runs.
pub(super) fn surface_kept_folder_with(
    preserved_location: Option<&str>,
    save_for_row: impl FnOnce(&str),
    alert: impl FnOnce(&str),
) {
    let Some(location) = preserved_location else {
        return;
    };
    save_for_row(location);
    alert(location);
}

/// Spec 2026-10-10 W9 and §9: after a sign-out, ONE alert names our folder, then macOS's. Our folder was recorded by
/// the keep step already; macOS's is saved for its row here, as before (the one route that tells the windows). Neither
/// path is logged.
pub(super) fn surface_sign_out_folders(app: &tauri::AppHandle, result: &Result<SignedOut, SignOutFailure>) {
    let (macos, ours) = sign_out_folders(result);
    match macos {
        Some(location) => save_kept_folder_for_row(app, location),
        None if ours.is_some() => emit_kept_folder_changed(app),
        None => {}
    }
    // Task 2191 (D11): on Windows "ours" is the folder files of unknown content were kept in.
    #[cfg(target_os = "windows")]
    let alert = match ours {
        Some(kept) => sign_out::copy::windows_kept_files_alert(&kept.folder, kept.files),
        None => sign_out::copy::sign_out_alert(None, macos),
    };
    #[cfg(not(target_os = "windows"))]
    let alert = sign_out::copy::sign_out_alert(ours.map(|kept| (kept.folder.as_str(), kept.files)), macos);
    let Some((title, message)) = alert else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Info)
        .show(|_| {
            tracing::debug!("kept-folder alert closed");
        });
    tracing::debug!("kept-folder alert shown");
}

/// The two folders a sign-out's result carries, whether it succeeded or failed after the removal (task 1882):
/// macOS's, then ours. It replaces the helper that named macOS's folder only.
pub(super) fn sign_out_folders(result: &Result<SignedOut, SignOutFailure>) -> (Option<&str>, Option<&KeptUnsent>) {
    match result {
        Ok(done) => (done.preserved_location.as_deref(), done.kept_unsent.as_ref()),
        Err(failure) => (failure.preserved_location.as_deref(), failure.kept_unsent.as_ref()),
    }
}

/// U26: "Show in Finder" reveals only the folder saved for the row.
pub(super) fn reveal_target<'a>(saved: Option<&str>, path: &'a str) -> Result<&'a str, String> {
    if saved == Some(path) {
        Ok(path)
    } else {
        Err(sign_out::copy::K3_TOAST.to_string())
    }
}

/// The row's folder (spec 2026-10-10 §9), or none.
#[tauri::command]
pub(super) fn kept_unsent_folder() -> Result<Option<String>, String> {
    sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)
}

/// The row's "Dismiss": forgets the folder it showed, never deletes it (§7.3).
#[tauri::command]
pub(super) fn dismiss_kept_unsent_folder(
    app: tauri::AppHandle,
    path: String,
) -> Result<finder_removal::DismissOutcome, String> {
    let outcome = sign_out::kept_record::dismiss_saved_kept_unsent_at(&DesktopConfig::path()?, &path)?;
    emit_kept_folder_changed(&app);
    Ok(outcome)
}

/// "Show in Finder" (spec 2026-10-10 §7.1): the opener's reveal, never `/usr/bin/open`. On macOS it fails only when
/// the folder is gone, which is what K3 says.
#[tauri::command]
pub(super) fn show_kept_unsent_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let saved = sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)?;
    let target = reveal_target(saved.as_deref(), &path)?;
    tracing::info!("kept folder reveal requested");
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|_| sign_out::copy::K3_TOAST.to_string())
}

/// Rebase re-review I2: told to every window once a kept folder was saved for the Settings › Sync row, so a Sync tab
/// that is open reads the folder again. A removal the reconciler runs in the background (after "Try again", or one
/// that finished after its time limit) saves the folder long after the tab's own read, and so does a Repair that the
/// compact window's Finder page started (device RB3-F1). No payload: the tab reads the folder through
/// `kept_unsynced_folder`, so the path is never sent to a window that did not ask for it. The frontend listens for
/// this name as `KEPT_FOLDER_CHANGED_EVENT`.
pub(super) const KEPT_FOLDER_CHANGED_EVENT: &str = "kept-folder-changed";

pub(super) fn emit_kept_folder_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Err(error) = app.emit(KEPT_FOLDER_CHANGED_EVENT, ()) {
        tracing::warn!(%error, "could not tell the windows that a kept folder was saved");
    }
}

/// Device RB3-F1 (lead ruling, 2026-10-10): the ONE place a kept folder is saved for the Settings › Sync row, and
/// every save of it tells the windows once it is saved, so a Sync tab that is open follows a Repair that another
/// window started (the compact window's Finder page), as it follows a sign-out or a late reconciler removal. The
/// callers are `surface_kept_folder` (the app-start sweep, the reconciler), `surface_sign_out_folders` (sign-out) and
/// macOS Repair, which has no alert of its own (1882 spec §5: the page's result line carries the folder). A save that
/// failed is logged without the path and the windows are told anyway; the tab reads the row again and finds what is
/// there. A source pin (`rb3_f1_every_save_of_the_rows_folder_goes_through_the_one_function_that_tells_the_windows`)
/// fails on any other route to `remember_kept_folder`.
pub(super) fn save_kept_folder_for_row<R: tauri::Runtime>(app: &tauri::AppHandle<R>, location: &str) {
    save_kept_folder_for_row_with(location, remember_kept_folder, || emit_kept_folder_changed(app));
}

/// [`save_kept_folder_for_row`] with its two steps passed in: `save` first, then `changed`, once each.
pub(super) fn save_kept_folder_for_row_with(location: &str, save: impl FnOnce(&str), changed: impl FnOnce()) {
    save(location);
    changed();
}

/// Saves the latest kept folder in `desktop.toml` (spec §5, "The kept-folder row"). Best-effort:
/// a failure is logged without the path, and the alert still names the folder.
///
/// Round 5: one load-change-save under the config-write lock (`DesktopConfig::update_at`), because
/// the app-start sweep runs this while the window may be saving a setting; two unserialized
/// saves share one `desktop.toml.tmp` and the later could drop the earlier one's change.
pub(super) fn remember_kept_folder(location: &str) {
    if let Err(error) = finder_removal::remember_kept_folder(location) {
        tracing::warn!(error = %error, "could not save the kept folder for Settings › Sync");
    }
}

/// Review I2: the folder Settings › Sync shows until the person dismisses it. `None` = no row.
#[tauri::command]
pub(super) fn kept_unsynced_folder() -> Result<Option<String>, String> {
    Ok(DesktopConfig::load()?.kept_unsynced_folder)
}

/// Review I2: the row's "Dismiss". Clears the saved folder only if it is still `path`, the one
/// the row showed. Round 5: it says whether it cleared it and which folder is saved now, so a row
/// that showed an older folder shows the newer one instead of vanishing. Under the config-write
/// lock, like the save above.
#[tauri::command]
pub(super) fn dismiss_kept_unsynced_folder(path: String) -> Result<finder_removal::DismissOutcome, String> {
    finder_removal::dismiss_saved_kept_folder(&path)
}

/// Put a session restored from the Keychain into memory. Deliberately no `bump_vault_epoch()`
/// (see its doc): the lock that preceded this already moved the epoch, and a cold start has no
/// cached storage summary to go stale. Split out of `unlock_vault` so a test can run the real
/// install after the real `lock_vault`; the Keychain read before it has no Linux backend.
pub(super) fn install_unlocked_session(
    _write: &SessionWrite,
    acct: &AccountRuntime,
    session: Session,
) -> Result<(), String> {
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    *guard = Some(session);
    Ok(())
}

/// The Keychain unlock's install, in its turn (lead ruling 1, Task 12): `turn` was captured before the Keychain was
/// read, and the session and `auth_present` go into memory only while no other session transition happened since.
pub(super) fn install_keychain_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    session: Session,
) -> Result<(), String> {
    let write = claim_session_write(acct, turn).map_err(|changed| changed.keychain_unlock_sentence().to_string())?;
    install_unlocked_session(&write, acct, session)?;
    set_auth_present(state, true);
    Ok(())
}

/// What `unlock_vault` says on a Mac that holds a session token and no vault key (the first password sign-in that was never
/// finished with the recovery phrase): the approved Set up this Mac sentence, which the Beebeeb window recognises to offer the
/// phrase (`isNoVaultKey`, pinned equal in `tests/signInModel.test.ts`). Without it the person read the Keychain's own
/// words, "unlock vault key from Keychain: secret not found". Windows and Linux keep what they showed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) const NO_VAULT_KEY_UNLOCK_ERROR: &str =
    "This Mac does not have your encryption keys yet. Restore them to continue.";

/// The same test the startup restore applies to this failure (`error.contains("secret not found")`, in
/// `restore_session_inner`): the token is there and the key is not.
pub(super) fn unlock_failure(error: String) -> String {
    #[cfg(target_os = "macos")]
    if error.contains("secret not found") || error == crate::auth_v2::key_owner::KEY_NOT_THIS_ACCOUNTS {
        return NO_VAULT_KEY_UNLOCK_ERROR.to_string();
    }
    error
}

/// Restore a Keychain-backed session into memory and start the engine for the
/// configured sync root. Keychain may show the OS unlock prompt depending on
/// the user's security settings. Until the user calls this command, hydration
/// and upload commands stay unavailable because there is no in-memory key.
#[tauri::command]
pub(super) async fn unlock_vault(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        let acct = state.active_account()?;
        let session_installed = acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .is_some();
        if session_installed {
            let started = start_engine_if_possible(
                app,
                &state,
                #[cfg(target_os = "windows")]
                &_transition,
            )
            .await;
            keys_arrived(&state, KeysFrom::Unlock);
            return started;
        }

        // The unlock is one session transition (lead ruling 1, Task 12): captured before the Keychain is read, and the
        // session goes into memory in its turn only.
        let mut turn = acct.session_generation();
        let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
        let session = load_session_from_keychain(acct.id.as_str(), email)
            .map_err(unlock_failure)?
            .ok_or_else(|| "Sign in before unlocking the vault.".to_string())?;
        install_keychain_session(&state, &acct, &mut turn, session)?;
        tracing::info!("vault unlocked from Keychain");
        let started = start_engine_if_possible(
            app,
            &state,
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await;
        keys_arrived(&state, KeysFrom::Unlock);
        started
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// Lock's engine stop, then the upload-staging purge it allows. Returns whether an engine stop on this account is
/// unconfirmed after it: this one, or an earlier one that left the slot empty (lead ruling T8-lockflag). Windows
/// refuses the lock instead, so there it is always `false`.
///
/// Task 1538 Codex P1: lock, like sign-out, clears the in-memory
/// session/master key right after this — an unconfirmed stop means the old
/// engine could still be alive and using it. No general local-file purge is
/// gated on this (lock keeps the account's Keychain session AND the regular
/// local file cache, so re-unlocking stays fast — there's nothing
/// cross-account to protect here), but it's still worth a loud warning rather
/// than a silent "we waited 3s and moved on". Windows refuses the lock.
///
/// Upload staging is emptied only when no engine can still be running: none
/// was, or its stop is confirmed, and no earlier stop is unconfirmed. The IPC
/// listener runs inside the engine's task; one that might still be serving
/// could take a request whose copy the purge just removed and refuse it as
/// `missing`, a definitive failure for the user's file. On an unconfirmed stop
/// the lock still goes ahead and the copies are left for the age-bound purge.
/// The caller holds the engine slot, so no new engine can start in between.
pub(super) async fn stop_engine_for_lock(
    acct: &AccountRuntime,
    engine_slot: &mut Option<runner::EngineRunner>,
    upload_staging: Option<&std::path::Path>,
) -> Result<bool, String> {
    // Lead ruling T8-lockflag: like sign-out, lock honours an earlier stop that was never confirmed.
    // It still clears the session, and it says to restart instead of reporting success.
    #[cfg(not(target_os = "windows"))]
    let mut engine_unconfirmed = acct.engine_stop_unconfirmed.load(Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    let engine_unconfirmed = false;
    #[cfg(not(target_os = "windows"))]
    if engine_unconfirmed {
        tracing::error!("vault lock: an earlier sync engine stop was never confirmed; restart Beebeeb");
    }
    if let Some(prev) = engine_slot.take() {
        if !stop_engine_in_slot(acct, prev).await.is_stopped() {
            #[cfg(target_os = "windows")]
            return Err("Vault lock failed: sync is still stopping. The vault is not locked. Retry locking; if it persists, restart Beebeeb.".into());
            // Lead ruling T8-lockflag: the consumed handle leaves an empty slot, so the flag (set by
            // `stop_engine_in_slot`) is what keeps every engine start refused (and sign-out) until a restart.
            #[cfg(not(target_os = "windows"))]
            {
                engine_unconfirmed = true;
                tracing::error!(
                    "engine did not confirm termination before vault lock cleared the in-memory session; restart Beebeeb"
                );
            }
        } else {
            tracing::info!("engine aborted on vault lock");
        }
    }
    purge_upload_staging_after_engine_stop("lock", !engine_unconfirmed, upload_staging);
    Ok(engine_unconfirmed)
}

/// The App Group upload-staging directory that sign-out and Lock empty.
/// `None` off macOS. Under `cargo test` it is never the real App Group
/// container: `None`, unless the test runs inside
/// [`with_upload_staging_for_test`] (a folder the test created).
pub(super) fn upload_staging_dir_for_session_purge() -> Option<std::path::PathBuf> {
    #[cfg(all(target_os = "macos", not(test)))]
    return Some(crate::ipc_socket::macos_upload_staging_dir());
    #[cfg(test)]
    return UPLOAD_STAGING_FOR_TEST.try_with(Clone::clone).ok();
    #[cfg(not(any(target_os = "macos", test)))]
    None
}

/// Test builds: run `body` with `staging` as the upload-staging folder the session purge empties, so a test drives
/// the real sign-out or Lock path against a folder it created. Only this task sees it.
#[cfg(all(test, unix))]
pub(super) async fn with_upload_staging_for_test<T>(
    staging: std::path::PathBuf,
    body: impl std::future::Future<Output = T>,
) -> T {
    UPLOAD_STAGING_FOR_TEST.scope(staging, body).await
}

/// Empty upload staging at sign-out or Lock, but only after a confirmed
/// engine stop (`engine_stopped`); otherwise leave it, logged, for the
/// age-bound purge. Best-effort: a failure is logged, never surfaced.
#[cfg(all(unix, any(target_os = "macos", test)))]
pub(super) fn purge_upload_staging_after_engine_stop(
    context: &'static str,
    engine_stopped: bool,
    upload_staging: Option<&std::path::Path>,
) {
    let Some(dir) = upload_staging else {
        return;
    };
    if !engine_stopped {
        tracing::warn!(
            context,
            "engine stop unconfirmed: upload-staging copies are left for the age-bound purge"
        );
        return;
    }
    crate::ipc_socket::purge_all_upload_staging_at(dir, context);
}

#[cfg(not(all(unix, any(target_os = "macos", test))))]
pub(super) fn purge_upload_staging_after_engine_stop(
    _context: &'static str,
    _engine_stopped: bool,
    _upload_staging: Option<&std::path::Path>,
) {
}

/// Lock clears all runtime key material and stops the sync daemon, but keeps
/// the Keychain session so the user can unlock again without re-entering their
/// recovery phrase. `Ok` means the lock happened; its `warning` says what of it could not be confirmed (FB-24).
#[tauri::command]
pub(super) async fn lock_vault(state: State<'_, AppState>) -> Result<SessionActionOutcome, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    let acct = state.active_account()?;
    // Task 12, lead ruling 1: no session transition that began before this lock writes anything from here on (the
    // session clear below ends any that begins while it runs).
    drop(end_sessions_in_flight(&acct, account::SessionTransition::Lock));
    #[cfg(target_os = "windows")]
    close_session_commands().await?;
    // Spec A (plan "Spec issues" 8): cancel any Finder check and hold BEFORE the engine stops, so the
    // reconciler cannot start an engine with keys this lock is about to clear. A reconciler that does
    // not acknowledge (it is stuck, or it has stopped) does not stop the lock: everything below still
    // runs, and the result says Finder could not be confirmed (lead rulings T8-⚠lock 1 and T7-⚠2 5).
    #[cfg(target_os = "macos")]
    let finder_lock = finder_lock_for(&state).await;
    #[cfg(target_os = "macos")]
    if let Err(error) = &finder_lock {
        tracing::warn!(%error, "the Finder reconciler did not acknowledge the lock; locking anyway");
    }
    let mut engine_slot = acct.engine.lock().await;
    #[cfg(all(test, target_os = "macos"))]
    signal_lock_holds_the_engine_slot();
    // The stop (and the upload-staging purge it allows) is `stop_engine_for_lock`'s; it says whether a stop on this
    // account is unconfirmed (lead ruling T8-lockflag), for the warning below.
    #[cfg_attr(target_os = "windows", allow(unused_variables))]
    let engine_unconfirmed = stop_engine_for_lock(
        &acct,
        &mut engine_slot,
        upload_staging_dir_for_session_purge().as_deref(),
    )
    .await?;
    // Task 1670 round 2: UNLIKE the general local-file cache above, the macOS
    // hydrate-cache holds nothing but ephemeral per-Finder-open staging
    // copies (never the user's regular offline files), so purging it on
    // every lock is always safe and always right — it does not touch
    // anything `unlock_vault` needs to stay fast.
    purge_macos_hydrate_cache("lock");

    #[cfg(target_os = "windows")]
    {
        windows_cf::revoke_callbacks()
            .await
            .map_err(|e| format!("Could not disconnect Cloud Files: {e}"))?;
        stop_own_data_proofs();
        windows_cf::wait_for_credential_release().await?;
    }

    // The session is cleared while this function still holds the engine slot (it cannot be written otherwise:
    // `clear_session_holding_slot` borrows the guard). Only after that is the slot released, and a start that
    // was waiting for it reads its keys under the slot and finds none.
    clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::Lock)?;
    #[cfg(not(target_os = "windows"))]
    drop(engine_slot);
    // Drop the cached account profile on lock too, so "locked == no profile"
    // stays honest; the next `unlock_vault` re-fetches.
    clear_cached_profile(&state);
    // Lock = clean slate: drop any in-flight 2FA challenge so an abandoned
    // partial token can't linger (and is zeroized) past a lock.
    if let Ok(mut guard) = state.pending_2fa.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    #[cfg(target_os = "windows")]
    {
        SESSION_COMMANDS.finish_close();
        AUTH_ATTEMPTS.finish_close();
    }
    set_auth_present(&state, keychain_session_present(acct.id.as_str()));
    // Everything this lock can clear is cleared, so it happened; what it could not confirm is a warning on that
    // success (FB-24). An engine that may still hold the key comes first: its fix is the more urgent one.
    #[cfg(not(target_os = "windows"))]
    if engine_unconfirmed {
        return Ok(SessionActionOutcome {
            warning: Some(ActionWarning::EngineStopUnconfirmed),
        });
    }
    #[cfg(target_os = "macos")]
    if finder_lock.is_err() {
        return Ok(SessionActionOutcome {
            warning: Some(ActionWarning::FinderLockUnconfirmed),
        });
    }
    Ok(SessionActionOutcome { warning: None })
}
