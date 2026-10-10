# 2026-10-09 — macOS: removing Beebeeb from Finder keeps files that never reached the server

**Status:** lead brief (task 1882, P0), written before the code.
**Date:** 9 Oct 2026
**Repo:** desktop (macOS only; Windows and Linux are unchanged)
**Task:** 1882
**Why a new spec and not a section of spec A:** this fix ships on `main` as a patch (0.8.12) while
spec A (`2026-10-06-macos-finder-setup-reconciler.md`) is still a branch. Spec A's removal paths
take the same rule when it rebases onto `main`, so this document is the one place both read.
**Amended 2026-10-10 (fix round 2):** the device rung and the review changed §2, §3, §4 and §5, and
added §8. Every change strikes the old text, keeps it visible and is signed under it.

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
  ~~`nil` means the system kept nothing.~~
  A URL does not mean that anything was kept. The header gives the type and nothing else (its doc
  comment is only "Remove a domain with options", `:236-238`). On the device, the helper added an
  empty domain and removed it again. macOS reported
  `~/Library/CloudStorage/Beebeeb-Drive (10-10-2026 10:52)`, and that folder did not exist
  afterwards. So the app checks the folder itself before it says anything (§4, §5).
  — lane impl-1882-r2, 2026-10-10, per lead ruling [1882-r2] (device K-F2, review I3)
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
| The cleanup inside Add to Finder, when a domain this attempt added does not come up in time (added 2026-10-10) | `macos_file_provider::install` → `remove()` → `beebeeb_fp_remove` |

The last row was missing from the first version of this table. That cleanup always used the
preserving mode (it goes through `remove()`), but it dropped the folder macOS reported. It now
surfaces it like the add rollback does (§5).
— lane impl-1882-r2, 2026-10-10, per lead ruling [1882-r2] (review M1)

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
folder is, and we do not guess: the device rung of task 1882 records the real location. ~~Beebeeb
never moves, opens, reads or deletes that folder: the files are the person's.~~ The app shows
exactly the path the system reported.

**Amended 2026-10-10 (device rung and round 2):**

- **Where it was.** On the device, two files that never uploaded were kept in
  `~/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)`. That is a dated name next to the
  domain's own location, not the location itself, so it does not occupy the path a later
  `addDomain` needs (review I4).
- **What Beebeeb does with the folder.** Beebeeb never moves, opens, reads or deletes the files
  in it: they are the person's. Right after the removal, it does two things, and only to decide
  whether to say anything (§5):
  - it checks that the folder exists;
  - it lists the folder until the first entry. It keeps no name and reads no file.
- **What the sandboxed app can see.**
  - **Existence:** the App Sandbox profile allows reading metadata everywhere
    (`/System/Library/Sandbox/Profiles/application.sb:496`, `(allow file-read-metadata)`). So
    the app can always tell whether the folder exists.
  - **Contents:** listing needs `file-read-data`. Outside the container, the sandbox grants that
    only through an extension that a URL carries.
  - **What the header says about the URL:** nothing. It does not say that `preservedLocation`
    is security-scoped. Compare `getUserVisibleURLForItemIdentifier`, whose comment says
    "Return the security scoped URL" and requires `startAccessingSecurityScopedResource`
    (`NSFileProviderManager.h:103`, `:120-121`).
  - **Where the check runs:** a path string never carries the scope ("a string-based path
    obtained from a security-scoped URL _does not_ have security scope", `NSURL.h:103`). So the
    bridge checks the folder on the `NSURL` macOS returned:
    `startAccessingSecurityScopedResource`, `stat`, the listing, then
    `stopAccessingSecurityScopedResource` if access was granted.
  - **If the listing is refused** for an existing folder, the app cannot tell whether it is empty.
    It then shows the folder (§5): it points at a folder that exists, never at nothing. It logs
    `contents_checked = false`, so the device rung shows which case it hit.
- ~~**Timing.** The check assumes macOS has filled the folder by the time the removal's completion
  handler runs. On the device, the folder existed afterwards; the rung must also confirm that the
  app saw it (§7).~~
  **Timing** (replaces the struck bullet; re-review D1). Nothing guarantees that macOS has made or
  filled the folder when the completion handler runs, and one device sample does not exclude a race.
  So the decision never hides a folder that exists:
  - A folder that exists is kept, empty or not, the same as one the app may not list.
  - A folder that does not exist is looked at again before the removal counts as "nothing kept":
    `KEPT_MISSING_RECHECKS` (4) looks, `KEPT_MISSING_RECHECK_INTERVAL` (250 ms) apart, about one
    second in all (`settle_kept_state` in `src-tauri/src/finder_removal.rs`). It stops at the first
    sight of the folder. The look is `stat`, which the sandbox always allows (above), so it works
    on a path alone. A removal that kept nothing pays that second; a removal that kept something
    does not.
  - A folder still missing after the last look is "nothing kept", and silent.

— lane impl-1882-r2, 2026-10-10, per lead ruling [1882-r2] (device K-F2, review I3, I4)

— lead ruling, 2026-10-10 (re-review D1), for the **Timing** bullet above

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
| Sign-out, every entry point | The app's own alert, title "Files kept on this Mac", body = the sentence, a blank line, the path. One alert per sign-out. Also when the sign-out fails after the removal: the alert comes first, then the error as before (added 2026-10-10, review I1). |
| Repair in Settings | ~~An inline status note under "Beebeeb in Finder" in the Sync tab: the sentence, then the path in mono. It sits next to the existing Repair note (`repairNote`), which is unchanged.~~ The Sync tab's kept-folder row (below), which the Repair refreshes. The existing Repair note (`repairNote`) is unchanged. (2026-10-10, review I2) |
| Repair in the compact window (Finder location page) | Appended to that page's existing result line: the sentence, then the path. |
| The Add-to-Finder rollback | The same alert as sign-out. The install failure is reported as before. |
| The cleanup inside Add to Finder (§3, added 2026-10-10) | The same alert as sign-out. The install failure is reported as before. |
| The app-start sweep | The same alert, once per folder kept. |
| Every removal above (added 2026-10-10, review I2) | Also the kept-folder row in Settings › Sync, until the person dismisses it (below). |
| `BeebeebFileProviderCtl remove` | Prints `preserved: <path>` on its own stdout, after `removed`. It is a developer tool. Since 2026-10-10 it prints that line only when files were kept by the rule below. ~~When macOS reported a folder that is missing or empty, it prints `nothing kept: macOS reported <path>, which is missing` (or `… empty`).~~ When macOS reported a folder that is still missing after the re-checks, it prints `nothing kept: macOS reported <path>, which is missing`. A folder that exists, empty or not, is `preserved:` like the app (re-review D1, lead ruling, 2026-10-10). |

Why an alert for sign-out rather than a line in the window: sign-out has four entry points
(Settings, the compact window's Account page, the menu item, and "Sign in again" from the
auth-expired banner or the Version Center), and two of them leave no window to write in: the menu
item has none, and "Sign in again" opens the sign-in window on top of the one it came from. All
four end in either the `clear_session` command or the menu handler, so the alert is raised in those
two places and no entry point can miss it. It is also the channel the menu's sign-out already uses
for its result on Windows (`DesktopMenuAction::SignOut` in `lib.rs`). Repair always starts from a
window that already shows Repair's result, so its sentence goes in that result.

~~**When nothing was kept:** the system reports no folder (`nil`), so there is no alert, no note and
no extra line, and every existing result reads exactly as before. A removal that fails is reported
as before (sign-out: logged, never shown, since sign-out must always complete; Repair: one
warning).~~

**When files count as kept** (amended 2026-10-10; it replaces the paragraph above):

| What the bridge finds | Outcome | What the person sees | Log (never the path) |
| --- | --- | --- | --- |
| No URL | Nothing kept | Nothing: no alert, no row, no extra line | debug, `reported = "none"` |
| ~~A URL whose folder does not exist~~ | ~~Nothing kept~~ | ~~Nothing~~ | ~~debug, `reported = "missing"`~~ |
| A URL whose folder does not exist, and still does not after the re-checks (§4, Timing) | Nothing kept | Nothing: no alert, no row, no extra line | debug, `reported = "missing"` |
| ~~A URL whose folder exists and is empty~~ | ~~Nothing kept~~ | ~~Nothing~~ | ~~debug, `reported = "empty"`~~ |
| A URL whose folder exists and is empty (it may not be filled yet, §4) | Kept | The sentence and the path, on every surface above | info, `preserved = true, contents_checked = true, empty = true` |
| A folder (or a single file) with at least one entry | Kept | The sentence and the path, on every surface above | info, `preserved = true, contents_checked = true, empty = false` |
| A folder that exists, but the listing (or the `stat`) is refused | Kept: the fallback in §4 | The sentence and the path | info, `preserved = true, contents_checked = false, empty = false` |
| A URL without a path ("removed; folder unknown") | Its own outcome: the domain is removed, but no folder can be named | Nothing: there is no folder to point at. Repair reports the domain as removed, with no warning | warn, "macOS reported kept files without a folder" |

— lead ruling, 2026-10-10 (re-review D1), for the two struck rows and the two rows that replace them

- **A removal that fails** is reported as before:
  - sign-out: logged and never shown, since sign-out must always complete;
  - Repair: one warning.
- **A reply that carries an error and a folder:** the folder is checked and surfaced like any
  other. The error is reported as before.
- **Repair fails after it removed the Finder location** (the config save fails; added 2026-10-10,
  1882 r4): the kept folder is shown first (the alert, and saved for the row if the save allows),
  as for a failed sign-out. ~~The error then read "Nothing was changed that you need to undo", which
  is false: the Finder location is gone.~~ The error now says so: "Beebeeb was removed from
  Finder" / "Repair couldn’t finish, so Beebeeb is no longer in Finder. Choose Add to Finder to add
  it back." (exact copy and the pre-removal case: `docs/specs/2026-10-02-macos-settings-dialogs.md`,
  Dialog 2). A Repair that fails before the removal, or whose removal itself failed, keeps the old
  note.

— lead ruling, 2026-10-10 (1882 r4)

— lane impl-1882-r2, 2026-10-10, per lead ruling [1882-r2] (device K-F2, review I3, M2, M3)

**The kept-folder row in Settings › Sync** (2026-10-10):

- **Saved locally, nowhere else.** The latest kept folder is saved in the app's own config
  (`desktop.toml`, `kept_unsynced_folder`). It is never written to a log and never sent anywhere.
  It is not part of the settings the Settings window saves, so no settings save can clear it.
  - A newer kept folder replaces the older one.
  - Dismissing clears it only if it is still the folder the row showed. The command says whether
    it did and which folder is saved now (`{ cleared, current }`), so a row that showed an older
    folder keeps showing the newer one instead of vanishing (round 5).
  - Every write to `desktop.toml` takes one process-wide lock: the sweep's save and the row's
    dismiss are a single load-change-save under it, so a settings save at the same moment cannot
    drop either change or tear the shared `desktop.toml.tmp` (round 5). Other load-modify-save
    callers of the config are not converted; they still replace the file with the copy they loaded.
- **When it appears:** after a sign-out, a Repair, the add rollback, the cleanup inside Add to
  Finder, or the app-start sweep, whenever files were kept by the rule above.
- **What it shows:** Settings › Sync, under "Beebeeb in Finder", a status note with ~~the sentence~~
  its own sentence (below), the path in mono and a "Dismiss" button. The path wraps (`white-space: normal;
  overflow-wrap: anywhere`) and is never cut off. The row stays until the person dismisses it. A
  tab switch, closing Settings or a restart does not clear it.
- **Its own sentence** (re-review D3):
  > Files that had not reached the server were kept in this folder:

  ~~The row showed the constant sentence above, "…your vault…".~~ The row outlives the sign-out
  that kept the files, so after an account switch another account reads it. "Your vault" would
  claim files that may be another account's, and could lead a person to drag them into this
  account's synced folder. The row says nothing about a vault, an account or a provider. The alert
  and the Repair result line appear straight after the removal and keep the sentence above.
  `KEPT_FOLDER_ROW_SENTENCE` lives once, in `src/macSettingsModel.ts`;
  `tests/finderPreservedFiles.test.ts` pins its text, and the Sync tab test checks the rendered
  row has no "your".
- **No "Show in Finder".** The headers do not say that a sandboxed app may reveal a path outside
  its container: `selectFile:inFileViewerRootedAtPath:` and `activateFileViewerSelectingURLs:`
  (`AppKit.framework/Headers/NSWorkspace.h:50`, `:53`) carry no sandbox statement. The only
  sandbox sentence in that header is about launch arguments (`:184`). Without that, the button is
  left out. A device check can add it later.
- **The design artefact** (`design/hifi/macos-settings-dialogs.html`) does not draw this row yet.
  Updating it is a design follow-up.

— lead ruling, 2026-10-10 (review I2)

— lead ruling, 2026-10-10 (re-review D3), for "Its own sentence" above

**Who draws the alert** (2026-10-10, device K-F1):

- **Mechanism.** The alert is `tauri-plugin-dialog`'s message dialog with no parent window. In
  `tauri-plugin-dialog` 2.7.2 with `rfd` 0.16, that path calls `CFUserNotificationDisplayAlert`
  on a background thread (`rfd` `backend/macos/utils/user_alert.rs`). macOS draws that alert in
  its own `UserNotificationCenter` process, not in a Beebeeb window.
- **What the device run showed.** No Beebeeb window held a dialog. But the system log has
  `UserNotificationCenter` taking the front 93 ms after the sign-out returned, and holding it for
  24.7 s. That was its only time in front that day.
- **What a device check must look for:** a window owned by `UserNotificationCenter`, not by
  Beebeeb. The app logs `kept-folder alert shown` (debug) when it raises the alert, and
  `kept-folder alert closed` when the person closes it.

— lane impl-1882-r2, 2026-10-10 (device K-F1)

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

## 8. Round 2 tests and device checks (added 2026-10-10)

**Tests** (in addition to §6):

- **Bridge, on a real disk (macOS):** the bridge's own folder check, through a test-only entry
  point that builds the same `NSURL` and runs the same function. The cases: a missing folder, an
  empty folder, a folder with one item, a folder with only a hidden item, and a single file.
- **Rust, every OS:** decoding every folder state into its outcome.
  - ~~Missing and empty are silent.~~ Missing is silent (after the re-checks); empty is kept (re-review D1).
  - One entry, or a refused listing, is kept.
  - A URL without a path is the "folder unknown" outcome.
  - An error that carries a folder keeps the folder.
  - An unknown state with a path never hides that path.
- **Sign-out:** the step after the removal fails (the Keychain clear) → the error carries the
  folder → the alert text is still produced.
- **Kept-folder record:**
  - a new folder replaces the old one;
  - dismissing a different path leaves the record alone;
  - dismissing the shown path clears it;
  - the record survives a settings save.
- **Frontend:**
  - the Sync tab shows the saved row on open, after a Repair, and after every Add to Finder
    attempt, whether it succeeded or failed (its cleanup or rollback may have kept files);
  - the row's sentence is neutral: it has no "your" (re-review D3, lead ruling, 2026-10-10);
  - a Repair that reported a folder shows it even when the saved record cannot be read back. A
    failed save is not this case: Repair then returns an error and the app's alert names the
    folder (re-review D5, P1);
  - it shows nothing when nothing is saved;
  - "Dismiss" sends the exact path; the row goes only if Rust cleared that folder, otherwise it
    shows the folder saved now (round 5);
  - the path element wraps.
- **Wiring pins:**
  - every removal path records the folder;
  - the alert logs `kept-folder alert shown` and `kept-folder alert closed`;
  - the cleanup inside Add to Finder surfaces its folder.

- **Round 3 (re-review D1), no sleeping in any test:**
  - an existing empty folder decodes to kept, on every surface (sign-out, Repair, a failed removal,
    the install cleanup);
  - a folder missing at the handler and present on a later look is kept (the look is injected, as
    is the wait; on macOS it also runs on a real disk, with the folder made during the wait);
  - a folder missing throughout is silent, after exactly `KEPT_MISSING_RECHECKS` looks totalling
    between 0.75 s and 1.5 s of waiting;
  - only a missing folder with a path is looked at again;
  - a source pin: both removal paths decode the bridge's reply in one place, and that place
    settles a missing folder first.

— lead ruling, 2026-10-10 (re-review D1)

**Device checks** (in addition to §7):

1. **Nothing dirty.** Sign out, and separately run Repair, with no local-only file. ~~Expect no
   alert, no row, and a debug line with `reported = "missing"` or `"empty"`.~~ Expect no alert, no
   row, and a debug line with `reported = "missing"`, about one second after the removal. An alert
   here, with `empty = true` in the info line, means macOS makes an empty folder for a clean
   removal: stop and report it (re-review D1, lead ruling, 2026-10-10).
2. **Something dirty.** Sign out with a file that never uploaded. Expect all of these:
   - a `UserNotificationCenter` window titled "Files kept on this Mac";
   - `kept-folder alert shown` in the log, then `kept-folder alert closed` after OK;
   - `contents_checked = true` or `false`. Record which: `false` means the URL carries no sandbox
     extension and the fallback was taken;
   - the Settings › Sync row, after signing in again;
   - the row still there after a tab switch and after a restart, until "Dismiss".
3. **Timing.** In check 2, the info line must say `preserved = true`. ~~A debug line with
   `reported = "missing"` while the folder exists afterwards means macOS fills the folder after
   the completion handler. Stop and report it: the check would then hide kept files.~~ A late
   folder is now looked for, for about one second (§4, Timing). A debug line with
   `reported = "missing"` while the folder exists afterwards means macOS makes it later than that:
   stop and report it. `empty = true` with `preserved = true` means the re-check caught the folder
   before it was filled, which is the case this guard exists for (re-review D1, lead ruling,
   2026-10-10).
