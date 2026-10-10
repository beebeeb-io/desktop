# 2026-10-09 — macOS: removing Beebeeb from Finder keeps files that never reached the server

**Status:** lead brief (task 1882, P0), written before the code.
**Date:** 9 Oct 2026
**Repo:** desktop (macOS only; Windows and Linux are unchanged)
**Task:** 1882
**Why a new spec and not a section of spec A:** this fix ships on `main` as a patch (0.8.12) while
spec A (`2026-10-06-macos-finder-setup-reconciler.md`) is still a branch. Spec A's removal paths
take the same rule when it rebases onto `main`, so this document is the one place both read.

## 1. Why

Every Finder-location removal on `main` used the remove-all form, with no removal mode:

| Where | Line on `main` (`5496e2a`) |
| --- | --- |
| `beebeeb_fp_remove` (sign-out, Repair, the Add-to-Finder rollback) | `src-tauri/macos/FileProviderBridge.m:82` |
| `beebeeb_fp_remove_domain_by_id` (the app-start sweep of other domains) | `src-tauri/macos/FileProviderBridge.m:380` |
| `BeebeebFileProviderCtl remove` (developer helper) | `BeebeebFileProviderTools/DomainControlTool.swift:73` (`NSFileProviderManager.remove(domain)`, the same call in Swift) |

In 0.8.11 every file created in the Finder location fails to upload (task 1873: the extension's
`createItem` ends in `-2005`, so the file stays on this Mac only). A sign-out, a Repair or the
helper's `remove` then deleted those files together with the location. On a device, three such
files were deleted this way (task 1873, "What happens"; the 1834 run's D4c step).

## 2. The API, quoted from the SDK

SDK: `MacOSX.sdk` 27.0 (`xcrun --show-sdk-path` →
`/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk`).
Header: `System/Library/Frameworks/FileProvider.framework/Headers/NSFileProviderManager.h`.

`NSFileProviderManager.h:20-31`:

```objc
typedef NS_ENUM(NSInteger, NSFileProviderDomainRemovalMode) {
    /// Don't keep any files that are current in the domain
    NSFileProviderDomainRemovalModeRemoveAll FILEPROVIDER_API_AVAILABILITY_V4_0_IOS = 0,

    /// Delete the domain from the system but keeps the at least all the
    /// dirty corresponding user data around.
    NSFileProviderDomainRemovalModePreserveDirtyUserData FILEPROVIDER_API_AVAILABILITY_V4_0 = 1,

    /// Delete the domain from the system but keeps all the downloaded
    /// corresponding user data around.
    NSFileProviderDomainRemovalModePreserveDownloadedUserData FILEPROVIDER_API_AVAILABILITY_V4_0 = 2,
} NS_SWIFT_NAME(NSFileProviderManager.DomainRemovalMode) FILEPROVIDER_API_AVAILABILITY_V4_0_IOS;
```

`NSFileProviderManager.h:231-239`:

```objc
/**
 Remove a domain.
 */
+ (void)removeDomain:(NSFileProviderDomain *)domain completionHandler:(void(^)(NSError *_Nullable error))completionHandler;

/**
 Remove a domain with options
 */
+ (void)removeDomain:(NSFileProviderDomain *)domain mode:(NSFileProviderDomainRemovalMode)mode completionHandler:(void(^)(NSURL *_Nullable_result preservedLocation, NSError *_Nullable error))completionHandler FILEPROVIDER_API_AVAILABILITY_V4_0_IOS;
```

- **The mode we use:** `NSFileProviderDomainRemovalModePreserveDirtyUserData` (`:26`).
- **The preserved-data URL:** the completion's `preservedLocation` (`:239`), `_Nullable_result`:
  `nil` means the system kept nothing.
- **Minimum macOS:** `FILEPROVIDER_API_AVAILABILITY_V4_0` is `API_AVAILABLE(macos(12.0))`
  (`NSFileProviderDefines.h:25`; the `_IOS` variant at `:27` is the same on macOS).
  In Swift the call is `NSFileProviderManager.remove(_:mode:completionHandler:)`; type-checked
  against this SDK it compiles at `-target arm64-apple-macos14.0`, and at `macos11.0` it fails with
  "'remove(_:mode:completionHandler:)' is only available in macOS 12.0 or newer".
- **Below the minimum:** there is no "below". Beebeeb requires macOS 14.0
  (`src-tauri/tauri.conf.json` `minimumSystemVersion`, `src-tauri/build.rs`
  `-mmacosx-version-min=14.0`, and the `macos14.0` targets in
  `scripts/build-fileprovider-extension.sh`), so the app does not install on a Mac that lacks the
  call. There is no fallback to the remove-all form and no `@available` branch: a fallback would
  bring the data loss back silently. If the minimum ever drops below 12.0, the source pin (§6) still
  demands the mode at every call, so the build has to answer the question instead of quietly
  falling back.
- The header does not say which mode the plain `removeDomain:completionHandler:` (`:234`) uses. The
  device evidence above shows it deletes files that never synced, which is what remove-all does.

**Why not `PreserveDownloadedUserData`:** it keeps every downloaded file, so a sign-out would leave
a decrypted copy of the person's vault on disk. Ending that access is what sign-out is for (the
local purge in `clear_session_impl`, task 1538). The dirty mode keeps only what has not reached the
server.

## 3. Every removal keeps what has not synced

Every call uses `PreserveDirtyUserData`:

| Removal | Code path |
| --- | --- |
| Sign-out: Settings → Account, the compact window's Account page, the menu's "Sign out", and "Sign in again" after a session ended (`forceReauth`). On `main` an account switch is a sign-out followed by a sign-in, so it takes this path too | `clear_session_impl` → `remove_file_provider_domain` → `beebeeb_fp_remove` |
| Repair: Settings → Sync → "Repair…", and the compact window's Finder location page | `reset_macos_integration` → `beebeeb_fp_remove` |
| The rollback when Add to Finder succeeded but the sync engine could not start | `install_finder_location` → `beebeeb_fp_remove` |
| The app-start sweep of domains that are not ours (task 1698) | `cleanup_stale_domains` → `beebeeb_fp_remove_domain_by_id` |
| `BeebeebFileProviderCtl remove` | `DomainControlTool.remove()` |

**What "dirty" covers, and what it does not.** macOS keeps what it still holds as not synced: an
item whose create or modify the extension failed or never finished. In 0.8.11 every Finder create
fails (1873), so every such file is kept.

**A limit this fix does not change (sign-out only).** The extension finishes a create or modify
as soon as the app has queued the upload (`WriteQueued`, `docs/IPC_PROTOCOL.md`). From then on
macOS counts the item as synced. Sign-out then purges the upload queue and its staged copies
(`purge_local_state_files`, a cross-account control from task 1538). So once 1873 is fixed, a file
that is still waiting in the upload queue at sign-out is neither in the queue nor kept by this
mode. Repair keeps the queue (`pending_operations_preserved`), so Repair is not affected. Owner:
the lead, as a follow-up task; spec A's R8 already removes the worst trigger ("Sign in again").

## 4. Where the kept files end up

macOS chooses the folder and reports it in `preservedLocation`. The header does not say where that
folder is, and we do not guess: the device rung of task 1882 records the real location. Beebeeb
never moves, opens, reads or deletes that folder: the files are the person's. The app shows
exactly the path the system reported.

## 5. What the person is told, and where

**The sentence (one constant, exact):**

> Files that hadn’t reached your vault yet were kept on this Mac, in this folder:

The folder's path follows on its own line, in mono wherever the surface has mono.

- It names no provider and makes no promise about uploading them later (with 1873 open, putting
  them back in the Finder location would not upload them either).
- The path is shown to the person only in the app's own UI. Logs never carry it: they say only
  that files were kept (`preserved = true`).
- The constant lives twice, once per language: `PRESERVED_FILES_SENTENCE` in
  `src-tauri/src/finder_removal.rs` and in `src/macSettingsModel.ts`. A test pins them equal.

**Where it appears:**

| Removal | Where the sentence appears |
| --- | --- |
| Sign-out, every entry point | The app's own alert, title "Files kept on this Mac", body = the sentence, a blank line, the path. One alert per sign-out. |
| Repair in Settings | An inline status note under "Beebeeb in Finder" in the Sync tab: the sentence, then the path in mono. It sits next to the existing Repair note (`repairNote`), which is unchanged. |
| Repair in the compact window (Finder location page) | Appended to that page's existing result line: the sentence, then the path. |
| The Add-to-Finder rollback | The same alert as sign-out. The install failure is reported as before. |
| The app-start sweep | The same alert, once per folder kept. |
| `BeebeebFileProviderCtl remove` | Prints `preserved: <path>` on its own stdout, after `removed`. It is a developer tool. |

Why an alert for sign-out rather than a line in the window: sign-out has four entry points
(Settings, the compact window's Account page, the menu item, and "Sign in again" from the
auth-expired banner or the Version Center), and two of them leave no window to write in: the menu
item has none, and "Sign in again" opens the sign-in window on top of the one it came from. All
four end in either the `clear_session` command or the menu handler, so the alert is raised in those
two places and no entry point can miss it. It is also the channel the menu's sign-out already uses
for its result on Windows (`DesktopMenuAction::SignOut` in `lib.rs`). Repair always starts from a
window that already shows Repair's result, so its sentence goes in that result.

**When nothing was kept:** the system reports no folder (`nil`), so there is no alert, no note and
no extra line, and every existing result reads exactly as before. A removal that fails is reported
as before (sign-out: logged, never shown, since sign-out must always complete; Repair: one
warning).

## 6. Tests

- **Source pin (Rust, runs on every OS):** reads every Objective-C and Swift source in the repo and
  fails on any `removeDomain:` without `mode:NSFileProviderDomainRemovalModePreserveDirtyUserData`,
  any Swift `NSFileProviderManager.remove(` without `mode: .preserveDirtyUserData`, and any
  remove-all call (`removeAllDomains`). It also requires a minimum number of removal sites, so a
  scan that matched nothing is a red, not a green. Mutation: drop the mode from one call → red.
- **Rust:** the bridge's reply → `DomainRemoval`; a kept folder reaches the sign-out report, the
  Repair result (`preserved_location`) and the alert text; nothing kept → `None`, no alert text;
  an error never invents a folder.
- **Frontend:** the Sync tab renders the sentence and the path after a Repair that reported a
  folder, and renders neither when it did not; the TypeScript constant equals the Rust one.

## 7. Device rung (lead; task 1882's Verification)

On a signed QA build of `main` with this fix: create a file in the Finder location (it fails to
upload, per 1873), then sign out. The file must still exist at the reported folder with the same
sha256, and the alert must name that folder. Repeat with Repair (the Sync tab note names it).
