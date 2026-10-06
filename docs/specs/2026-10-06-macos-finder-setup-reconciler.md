# 2026-10-06 — macOS: Beebeeb in Finder just works (spec A of 3)

**Status:** design approved section by section by Guus on 2026-10-06; written spec awaiting his review.
**Amended:** 2026-10-06 after the plan review, for rulings R8–R11 and the plan's findings (`docs/superpowers/plans/2026-10-06-macos-finder-setup-reconciler.md`, "Spec issues found"). Each amendment is inline: the original is struck through and kept readable, and the replacement sits beneath it. — lead, 2026-10-06 (plan review)
**Date:** 6 Oct 2026
**Repo:** desktop (macOS only; Windows and Linux behaviour is unchanged)
**Task:** allocated together with the implementation plan.
**Siblings:** spec B (the popover becomes the only macOS surface) and spec C (launch and first run become "sign in, and that's it") follow this one, each with its own spec, plan and review.
**Supersedes:** the "Finder not added" state and its "Add to Finder" action in the 1683 popover design (see Rulings, R5).

## 1. Why

Guus, 2026-10-06, on desktop 0.8.11:

> "The app sucks and doesnt work, install location fails (after update), then drive status still shows this, but it should just show to me to login and thats it."

"Install location" is onboarding's **Install the Finder location** step. This spec makes that step disappear: once you are signed in and this Mac has your keys, Beebeeb puts itself in Finder, keeps itself there across launches and updates, and says one honest sentence when macOS really stops it.

## 2. What we found (verified on Guus's Mac, 2026-10-06)

1. **The failure.** `fileproviderd` refused every add attempt from 0.8.11 (pid 31804) with the fault *"attempting create a domain root at `<FPFS>/B…b`, but that path already exists and is owned by a different provider `io.beebeeb.desktop.FileProvider/…domain`"*. The app made 11 attempts between 15:04:31 and 15:05:09 and then showed a generic error.
2. **The owner of the path.** `~/Library/CloudStorage/Beebeeb-Beebeeb` (created 18 May 20:48) carries `com.apple.file-provider-domain-id: io.beebeeb.desktop.FileProvider/io.beebeeb.desktop.domain`. Its extension is no longer installed (`pluginkit` lists only `io.beebeeb.app.FileProvider`), and `ls` on the folder times out. It is an orphan from our own app before the bundle rename: the app was `io.beebeeb.desktop` until `b9da3be` (18 May 2026).
3. **Why it collides now.** A domain's folder is `<app name>-<domain display name>`. Before 0.8.9 our domain was named "Drive" (`Beebeeb-Drive`). Task 1698 (`bb42d29`, shipped in 0.8.9) renamed it "Beebeeb", so it now asks for exactly the orphan's folder.
4. **Why our domain was gone.** Sign-out removes our domain on purpose (`clear_session_impl`, lib.rs ~1846–1857: "a logged-out machine has no live File Provider domain"). After a sign-out, the next sign-in has to add it again, and that add hits the orphan. `BeebeebFileProviderCtl status` reported `missing`.
5. **Why nothing cleaned it up.** `NSFileProviderManager.getDomains` returns only the calling provider's own domains, so the 1698 stale-domain sweep can never see `io.beebeeb.desktop.FileProvider`. `fileproviderctl` has no remove command.
6. **No way to diagnose it on a user's Mac.** The app logs only to stdout (`tracing_subscriber::fmt()`, lib.rs ~8736), which is lost when macOS launches the app. The evidence above came from the system log, which only reaches back about four hours.
7. **A revoked session plus "Sign in again" throws away edits that have not uploaded (data loss, exists today).** `forceReauth` (`src/desktopApi.ts:1030`) calls `clearSession()` and then opens onboarding. `clear_session` is the full sign-out: it purges the operation queue and the decrypted cache (`lib.rs` ~1793–1844) and removes the Finder domain. Verified in code by the lead. Ruling R8 replaces this path. — lead, 2026-10-06 (plan review)
8. **Local data on this computer is not yet bound to the account that created it (hardening).** `state.db` and the files it points at carry no record of their account. Ruling R10 adds that binding: a different account never reuses this data (§5.6). Background: private workspace task 1835. — lead, 2026-10-06 (plan review)

### Correction: tasks 1696 and 1698

1698's comment in `src-tauri/src/macos_file_provider.rs` (the `DOMAIN_IDENTIFIER` doc) says the zombie `io.beebeeb.desktop.FileProvider` domain "keeps its 'Beebeeb-Drive' volume … until a SIGNED context enumerates and removes it". **That was wrong on two counts.**

- The zombie's folder is `Beebeeb-Beebeeb`, not `Beebeeb-Drive`.
- No signed `io.beebeeb.app` context can enumerate it, because `getDomains` is per provider.

The 1696 forensics line "we registered 'Drive' while the system showed 'Beebeeb'" was reading the **zombie's** name, so 1698 renamed our domain to match the zombie and created the collision. The implementation corrects that comment by adding the correction beneath it, leaving the original claim visible, per the honesty conventions.

**Who is affected.** Only Macs that ran a build from before 18 May. `io.beebeeb.desktop` never shipped publicly (the first public macOS release was 0.8.5, task 1608, 29 Sept), so in practice that means Guus's Mac and possibly Bram's. The bug class is still real for everyone: an add failure gets no specific reason, no retry limit, and no log.

## 3. Rulings recorded (Guus, 2026-10-06, in the brainstorming session)

- **R1: three separate specs.** "the seperate specs, prefer slow and quality over speed and quantity". Order: A (this spec), then C, then B.
- **R2: a revoked session keeps Beebeeb in Finder.** "Keep it, ask to sign in". Files stay listed, files not on this Mac cannot open until sign-in, the app asks for sign-in again, and edits made meanwhile are kept and upload after sign-in. A sign-out you choose still removes the domain.
- **R3: device checks run on Guus's own Mac account.** "My own Mac account" / "you're running on a mac right now". His real session gets signed out during the checks, and he signs back in at the end.
- **R4: one Finder reconciler in Rust** (approach 1). "looks good, lets go with it". This was chosen over calling `install_finder_location` automatically from the frontend (approach 2) and over managing the domain from the extension or a launch agent (approach 3, impossible: only the containing app can add or remove its domain).
- **R5: no "Add to Finder" anywhere.** "yep" to Section 1, which drops the 1683 "Finder not added" state and its "Add to Finder" button. "Try again" survives only inside a failure. This supersedes the approved 1683 artifact for that state, and the artefact is updated first (section 12).
- **R6: the orphan is removed with a one-off helper.** "Yes, I'll make the profiles". Guus creates two development provisioning profiles for the old IDs when the cleanup step comes up.
- **R7: device check D8 may sign a test account into production** to exercise the real alpha update path. This was approved with Section 6 ("perfect"). It is the only production action this spec authorizes.
- **R8: re-sign-in in place** (2026-10-06, after the plan review). Question: "Sign in again" after a revoked session currently signs you out fully (Finder entry removed, edits not yet uploaded thrown away); how should it work? Answer: **"Re-sign-in in place, in spec A (Recommended)"**. The option he chose: "Sign in again as the same account and only the session token is replaced. Finder, keys, cache and pending edits all stay, and the edits upload afterwards. Signing in as a different account is an account switch: full sign-out, with a warning if edits haven't uploaded yet. Adds auth code to A, with a security review." — lead, 2026-10-06 (plan review)
- **R9: the vault key stays after a remote revocation** (2026-10-06). Question: keep the vault key on this Mac after its session is revoked, so the same account signs in again without its recovery phrase? Answer: **"Keep the key, review decides (Recommended)"**. The Mac keeps its key. The change merges only if the security review agrees; if the review disagrees, the decision returns to Guus. — lead, 2026-10-06 (plan review)
- **R10: local data is bound to the account that created it** (2026-10-06). Answer: **"Fold into 1834"**: this spec also owns that binding. A different account never reuses this computer's local data. Behaviour in §5.6; background in private workspace task 1835. — lead, 2026-10-06 (plan review)
- **R11: Windows stays fail-closed** (2026-10-06), on the Windows gap of R10. Answer: **"Refuse now, escape hatch as its own task (Recommended)"**. This spec keeps Windows refusing (§5.6). An explicit "Discard and switch" escape hatch is private workspace task 1837, verified on Windows CI and a Windows PC. — lead, 2026-10-06 (plan review)

## 4. Goal and non-goals

**Goal.** On a Mac, after sign-in, Beebeeb appears in Finder with no further click, stays there across launches, updates and revoked sessions, and leaves only when you sign out. When macOS blocks it, you get one sentence and one action, the app does not retry in a loop, and the reason is in a log file.

**Non-goals (handed on):**
- Which window or popover shows sign-in, and the launch behaviour → spec C.
  The same-account re-sign-in itself (R8) is in this spec. — lead, 2026-10-06 (plan review)
- Removing the old compact window ("Drive status") and the hidden webviews (the source of about 1.7 M WebKit log lines in two hours on 0.8.9) → spec B.
- Windows CFAPI and Linux FUSE registration: unchanged. `install_finder_location` stays for them.
- A "Move to Applications" button that moves the app for you: not built. The app tells you, and you move it.

## 5. Behaviour

### 5.1 What the reconciler wants

| Situation | Wanted |
|---|---|
| Signed in and this Mac has your keys (`logged_in && vault_unlocked`) | **present** |
| Signed in, session revoked (`auth_expired`) | **present** (R2) |
| Signed in, keys missing (the recovery-phrase step) | no action: neither add nor remove |
| Signed out by choice | **absent** |
| "Repair…" in Settings (`reset_macos_integration`) | **absent, then wanted again**: remove, then one fresh check, so a signed-in Mac ends with Beebeeb back in Finder |
| Signed out without choosing it: the startup probe got a 401 for a revoked token (— lead, 2026-10-06 (plan review)) | **no action**: Beebeeb stays in Finder (R2). A persisted `finder_signed_out_by_choice` tells this apart from a sign-out by choice; only that one means absent |
| Signed in again as the **same** account after a revoked session, R8 (— lead, 2026-10-06 (plan review)) | **present, nothing removed**: only the session token is replaced. Keys, cache, queue and Finder stay, and the queue uploads afterwards |
| Signing in as a **different** account, R8 (— lead, 2026-10-06 (plan review)) | an account switch: the pending-edits warning first, then a full sign-out (**absent**, queue and cache purged), then a fresh sign-in (**present**) |
| Vault locked (`lock_vault`) (— lead, 2026-10-06 (plan review)) | **no action**: a check in flight is cancelled before the engine stops, and nothing starts again until keys arrive |

### 5.2 What it reports

It reuses the existing `surfaces::phase::FinderSetup` the popover reducer already consumes, so there is no second vocabulary. `Failed` gains a reason (section 6). Because `PopoverSnapshot` is `Copy`, the reason travels as a separate `finder_reason: Option<FinderFailureReason>` field.

| State | Meaning |
|---|---|
| `Adding` | a check is in flight, including its silent retries (popover state f1, "Adding Beebeeb to Finder…") |
| `Ready` | registered, enabled and stable |
| `UserDisabled` | turned off in System Settings; add is never called while it is off |
| `Failed(reason)` | retry budget spent or not retryable; one sentence and one action |
| `Missing` | only the instant before the first check, or while the wanted state is "no action"; never a state a person waits in |

### 5.3 What starts a check

1. **App launch**, including the relaunch after an update. This also covers "wanted absent but still registered", for example after a sign-out whose removal failed.
2. ~~**Keys arrive on this Mac**, from every path: `desktop_login` returning unlocked, `desktop_login_2fa`, the browser sign-in handoff (`browser_login::run_handoff`), and `desktop_unlock_with_recovery_phrase`.~~
   **Keys arrive on this Mac**, from every function that puts the master key in memory: the browser handoff (`apply_session`), `desktop_unlock_with_recovery_phrase`, `unlock_vault` (the Keychain unlock after a lock), and a same-account re-sign-in that loads the key from the Keychain (R8). The startup restore is covered by trigger 1, which fires after the restore has finished. `desktop_login` and `desktop_login_2fa` store only a token and never put keys in memory. A test keeps this list complete. — lead, 2026-10-06 (plan review)
3. **While `UserDisabled`:** the enable poll sees `userEnabled` flip to true (section 7).
4. **"Try again"**, which starts a fresh retry budget.
5. **Sign-out by choice** (`clear_session`) sends the reconciler a remove. It cancels any check in flight and removes the domain, so add and remove can never race.
   This runs **first** in `clear_session`, before the engine stops, so no check can start an engine with the keys being cleared. It persists `finder_signed_out_by_choice`, which the next sign-in clears. An account switch (R8) is this same sign-out. — lead, 2026-10-06 (plan review)
6. **"Repair…"** (`reset_macos_integration`) sends remove-then-check. The reconciler cancels any check in flight, removes the domain, then runs one check with a fresh retry budget.
7. **Lock** (`lock_vault`) cancels any check in flight and holds until keys arrive again. It is acknowledged before the engine stops. It never removes. — lead, 2026-10-06 (plan review)

A revoked session is deliberately **not** a trigger.
Neither is a same-account re-sign-in (R8): it replaces only the session token, so the reconciler has nothing to add or remove. Keys that the re-sign-in loads from the Keychain count as trigger 2, which confirms a domain that is already there. — lead, 2026-10-06 (plan review)

### 5.4 One check at a time

There is exactly one reconciler task. A trigger that arrives while a check is running sets a "check again" flag, so any number of triggers during a check produce at most one follow-up check.

### 5.5 One check

```mermaid
stateDiagram-v2
    [*] --> Missing
    Missing --> Adding: trigger and wanted present
    Adding --> Ready: registered, enabled, stable
    Adding --> UserDisabled: userEnabled false or -2011
    Adding --> Adding: transient failure, attempts remaining
    Adding --> Failed: budget spent, or not retryable
    Failed --> Adding: Try again, keys arrive, or next launch
    UserDisabled --> Adding: userEnabled flips true
    Ready --> Adding: launch finds it missing
    Ready --> Missing: sign-out or Repair removes it
    Failed --> Missing: sign-out or Repair
    UserDisabled --> Missing: sign-out or Repair
```

Steps:
1. Read macOS's own state (`domain_user_enabled_state`).
2. If registered and disabled, stop at `UserDisabled`.
3. If registered and enabled, confirm it is stable (short wait), then `Ready`.
4. If not registered, check the launch location (section 6.2), call `addDomain`, re-read `userEnabled`, and wait up to 10 s for stabilization. The result is `Ready`, `UserDisabled`, or a classified failure that the retry policy turns into another attempt or `Failed`.
5. ~~On `Ready`, start the sync engine and save the sync folder. `install_finder_location`'s macOS arm does this today (`start_engine_for_pending_finder_install` / `persist_sync_root_and_start_engine`); it moves here.~~
   The sync engine starts **before** `addDomain`, reusing one that is already running, because the extension talks to the daemon's socket and stabilization needs it. This is what `start_engine_for_pending_finder_install` does today. On `Ready` the sync folder is saved and the engine keeps running. When the check ends in `Failed` or `UserDisabled`, an engine this check started is stopped. — lead, 2026-10-06 (plan review)
6. On every transition, emit `finder-setup-changed` so surfaces refresh without polling, and write one log line (section 8).

### 5.6 Local data belongs to one account (R10)

Added for ruling R10. — lead, 2026-10-06 (plan review)

- The local data on this computer is `state.db` (the queue, staged uploads, upload sessions, file rows, Finder anchors, activity) plus the files it points at (staged payloads, the decrypted cache). Its owner is recorded inside `state.db`: the server user id and the account email.
- Before any sync engine starts, the session's account is compared with that owner. The user id decides when both sides know it; otherwise the email decides, ignoring case.
  - **Same account:** everything is kept (R8).
  - **A different account, or local data with no recorded owner:** the local data is reset before anything starts (the sign-out purge, then every remaining row), and the new account becomes the owner. On a Mac, Beebeeb is then repaired in Finder so that Finder lists only the new account's files.
  - **An owner that cannot be compared with the session:** nothing starts and nothing is deleted.
  - **Windows:** a reset is refused instead. Sync does not start, and the person signs out and in again, because Windows sign-out never discards unsent changes silently.
- A failed reset, or a failed sign-out purge on macOS or Linux, stops: no engine starts, and sign-out reports the failure instead of completing.
- Upgrade: local data from before this binding is adopted at startup by the account this computer's Keychain still names. A same-account re-sign-in (R8) records the owner too.
- A sign-out by choice forgets the owner and also deletes staged uploads.
- A sign-in counts as a first sign-in only when nothing of a previous account remains: no recorded owner, no Keychain email, no vault key, no cached profile, and no queued or staged data. Anything else is compared, and an account that does not match takes the switch (R8). The switch leaves no vault key and no account email behind, or it stops. The previous account's key is never used for, or left next to, another account. (Codex review of the plan, PR #113.) — lead, 2026-10-06 (plan review)

## 6. Failure reasons, copy and actions

### 6.1 The bridge returns structured errors

`src-tauri/macos/FileProviderBridge.m` returns **(error domain, code, message)** instead of one flattened string, and Rust receives `FpError { domain, code, message }`. Classification reads only `domain` + `code`. The message goes only to the log, redacted. The substring matching in `classify_finder_install_error` (lib.rs ~2106), which matched English fragments of our own copy, is deleted on macOS.

### 6.2 The table

Codes are from the macOS 27 SDK header `FileProvider.framework/Headers/NSFileProviderError.h` on the build Mac.

| Reason | macOS signal | Retried silently | Copy | Action |
|---|---|---|---|---|
| `extension_loading` | `NSFileProviderErrorDomain` -2001 ProviderNotFound, -2003 OlderExtensionVersionRunning, -2004 NewerExtensionVersionFound, -2012 ProviderDomainTemporarilyUnavailable, -2014 ApplicationExtensionNotFound | yes | while retrying: "Adding Beebeeb to Finder…"; after the budget: "macOS hasn't finished loading Beebeeb's Finder extension." | Try again |
| `user_disabled` | -2011 DomainDisabled, or `userEnabled == false` (this becomes the `UserDisabled` state, not `Failed`) | no | "Beebeeb is turned off in System Settings." | Open System Settings (`open_login_items_and_extensions_settings`; recovers by itself) |
| `not_in_applications` | -2002 ProviderTranslocated, or the launch-location check | no | "Beebeeb is running from the disk image. Move it to Applications, then open it again." | Show in Finder |
| `folder_taken` | the domain root path is owned by a different provider. ~~**The NSError domain and code are captured by device check D0** and written into this cell before the classifier for this row merges.~~ Very likely `NSCocoaErrorDomain` 516 (`NSFileWriteFileExistsError`), possibly with an underlying `NSPOSIXErrorDomain` 17: Guus's 0.8.11 saved exactly "…(NSCocoaErrorDomain 516)" for this collision at 2026-10-06 13:05:09Z. The classifier ships with this pair, and device check D0 still confirms it (and records the underlying error) before it merges. — lead, 2026-10-06 (plan review) | no | "An older Beebeeb installation still holds Beebeeb's place in Finder. Contact support and we'll help you clear it." | Copy details |
| `signing` | ~~entitlement, app-group or provisioning errors (exact codes listed in the plan from the bridge's existing provisioning paths)~~ No code yet: the bridge has no provisioning-specific path, so the code set ships **empty** and a test pins it as empty. Until a code is captured, such an error classifies as `unknown`. — lead, 2026-10-06 (plan review) | no | "This copy of Beebeeb can't add itself to Finder. Download it again from beebeeb.io." | Copy details |
| `timeout` | added, but never stable within the wait | yes | "macOS didn't finish adding Beebeeb to Finder." | Try again |
| `unknown` | anything else | once | "Beebeeb couldn't be added to Finder." | Try again |

Rules:
- **One sentence and one action** (1683 ruling 7). A failure that gates Finder is inline, never a toast.
- **"Copy details"** copies: app version, macOS version, reason, NSError domain and code, attempt count, and the time of the last attempt. Never paths, file names or the email.
- **All copy lives in one frontend module** (`src/finderSetupCopy.ts`), keyed by reason, with a test pinning every string above. No surface writes its own.
- `-2005` (CannotSynchronize) and `-2013` (ProviderDomainNotFound) are in no row, so they classify as `unknown`. — lead, 2026-10-06 (plan review)
- Copy uses the typographic apostrophe (’), as all product copy does. — lead, 2026-10-06 (plan review)
- "Show in Finder" reveals the running app bundle through a new command, `finder_setup_show_app`. — lead, 2026-10-06 (plan review)
- **The launch-location check** (`launch_location(bundle_path)`) is pure:
  - bundle under `/Applications` or `~/Applications` → OK
  - a path containing `/AppTranslocation/`, or starting with `/Volumes/` → not OK
  - anywhere else → proceed and let macOS decide (a -2002 is still classified)
  It runs at launch and is exposed in `finder_setup_state` before sign-in, so spec C can show it before anyone types a password. Spec A only provides the detection and the copy.

## 7. Retry policy

- **Transient reasons** (`extension_loading`, `timeout`): at most **4 attempts**, starting 0 s, 5 s, 15 s and 45 s after the check starts. An attempt still in its ≤10 s stabilization wait when the next start time arrives delays that attempt until it finishes. A check therefore ends within about 55 s plus any such delay, and the person sees `Adding` throughout. After the 4th failure the state becomes `Failed(reason)`.
- **`unknown`**: one silent retry after 5 s, then `Failed`.
- **Not retryable** (`folder_taken`, `signing`, `not_in_applications`): `Failed` straight away.
- **After `Failed`**, nothing retries automatically until the next launch, keys arriving, or "Try again". Bringing the app to the front does not retry.
- **`UserDisabled` poll:** a read-only `userEnabled` check every 3 s, only while a Beebeeb window is visible, plus once when the app becomes active. It never calls add. The flip to true triggers exactly one check.
- **The schedule and the clock live in one place** (the reconciler's policy struct) and are injected in tests.

## 8. Lifecycle log

- ~~**File:** `~/Library/Logs/Beebeeb/lifecycle.log`, visible in Console.app. It rotates at 1 MB (`lifecycle.log` → `.1` → `.2`) and keeps 3 files. It is our own small writer, with no new dependency.~~
  **File:** `~/Library/Logs/Beebeeb/lifecycle.log` from the app's point of view. The app is sandboxed, so on disk it is `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log`. It rotates at 1 MB (`lifecycle.log` → `.1` → `.2`) and keeps 3 files. It is our own small writer, with no new dependency. Whether Console.app lists it is checked on the device (D0), not assumed. — lead, 2026-10-06 (plan review)
- **Closed vocabulary.** Each line is one typed event:
  - launch: app version, macOS version, launch location
  - trigger received
  - reconciler transition: from, to, reason, NSError domain and code, attempt n of N
  - sign-in completed or signed out: no email, no account id
  - update downloaded or installed: from and to versions
- **The only free text** is the NSError message, passed through `diagnostic_redaction::redact_for_export` with the state DB's known names, so paths become `[path]` and unknown words become `[name]`.
- **It is not a `tracing` sink.** The general `tracing` stream (engine errors embed paths and file names) never reaches the file.
- **It contains no user content, so it survives sign-out.** That is what keeps a failed sign-in diagnosable.
- **"Copy details" and the 1685 support bundle include its last 200 lines.**
- Stdout `tracing` stays for dev runs.

## 9. Units and interfaces

Each unit has one purpose and can be tested on its own.

| Unit | Purpose | Interface | Depends on |
|---|---|---|---|
| `finder_setup::core` (new, pure) | decide the next state and effects | `step(state, event, observation, now) -> (state, Vec<Effect>)` | nothing (no OS, no clock, no I/O) |
| `finder_setup::policy` (new, pure) | retry schedule and reason classes | `next_attempt_at(reason, attempt, started) -> Option<Instant>`; `classify(&FpError) -> Reason` | nothing |
| `finder_setup::driver` (new) | the single reconciler task: receives events, runs observations and effects, merges triggers, owns the timer | `FinderSetupHandle::send(Event)`; `state() -> FinderSetupView` | core, policy, bridge, engine start, log |
| `macos_file_provider` + `FileProviderBridge.m` (changed) | talk to `NSFileProviderManager` | the existing calls return `Result<_, FpError>` | FileProvider.framework |
| `finder_setup::launch_location` (new, pure) | where the app runs from | `launch_location(&Path) -> LaunchLocation` | nothing |
| `lifecycle_log` (new) | the file in section 8 | `lifecycle_log::event(LifecycleEvent)` | redaction |
| `reauth` (new, pure) | R8: is this sign-in the same account, a different one, or a fresh Mac | `sign_in_kind(local, user_id, email) -> SignInKind` | nothing (— lead, 2026-10-06 (plan review)) |
| `account_binding` (new, pure) | R10: may this session use the local data, or must it be reset first | `decide(owner, session, has_local_data) -> Binding`; `bind_before_engine_start(…)` | nothing (— lead, 2026-10-06 (plan review)) |

**Tauri commands on macOS:**
- `finder_setup_state` (read; replaces `finder_location_state`'s macOS use)
- `finder_setup_retry` ("Try again")
- `finder_setup_copy_details`
- `open_login_items_and_extensions_settings` (exists)
- `finder_setup_show_app` ("Show in Finder" for `not_in_applications`) — lead, 2026-10-06 (plan review)
- `open_reauth_window` (R8: "Sign in again" opens sign-in in place; nothing is cleared first) — lead, 2026-10-06 (plan review)
- `desktop_login` / `desktop_login_2fa` return what the sign-in became: a fresh sign-in, a re-sign-in in place, or `account_mismatch` with the pending-edit count. On a mismatch nothing local changes. — lead, 2026-10-06 (plan review)

The macOS frontend no longer calls `install_finder_location`, `continue_without_finder_location` or `finder_domain_user_enabled`; the poll moves into the driver. The commands stay registered for Windows and Linux. `PopoverRuntime`'s `finder_adding_guard` is replaced by the driver's published state.

**Config (`desktop.toml`):**
- The free-text `finder_install_last_error` and the string `finder_install_status` stop being written on macOS. Truth comes from macOS on every check.
- What persists is `finder_last_failure = { reason, domain, code, at }`, for "Copy details" across a relaunch.
- `finder_signed_out_by_choice`: set by a sign-out by choice and cleared by the next sign-in. A startup 401 does not set it, so Finder stays (R2). — lead, 2026-10-06 (plan review)
- ~~`signed_in_user_id`: the server user id of the account on this Mac. It is written at sign-in, backfilled by the startup probe, and cleared by a sign-out by choice. It is the identity R8 compares.~~ — lead, 2026-10-06 (plan review)
  The account identity is not a config key: it is the owner record inside `state.db` (R10, §5.6), which R8 compares as well. — lead, 2026-10-06 (plan review)
- Old keys must still load: they are ignored, never an error, and a test loads a 0.8.11 `desktop.toml`.

## 10. Frontend changes in this spec (macOS)

Kept to what makes A shippable on its own. Spec C later deletes the onboarding step, and spec B the old window.

- **`Onboarding.tsx`, Finder step:**
  - no "Install Finder location" button
  - shows `Adding`, then advances by itself on `Ready`
  - on `Failed` / `UserDisabled`, shows the one notice and one action from `finderSetupCopy.ts`
- **`MacSettings.tsx` Sync tab, `pages/SyncFolder.tsx`, `pages/Status.tsx`:**
  - read `finder_setup_state`
  - no "Install in Finder" on macOS
  - "Try again" only in `Failed`
  - `finderInstallCard.ts`'s macOS classifiers are replaced by the one reason-keyed copy module
- Subscribe to `finder-setup-changed` instead of polling `finder_location_state` every 3 s.
- The macOS main window's integration panel (`windows/views/SettingsView.tsx`, which called `install_finder_location` on macOS) reads `finder_setup_state` too. — lead, 2026-10-06 (plan review)
- **R8:** on macOS, "Sign in again" (`AuthExpiredBanner`, VersionCenter) opens sign-in in place and never calls `clear_session`. A same-account sign-in closes the window and sync resumes. A different account gets the warning, with the pending-edit count, and "Sign out and switch" / "Cancel". Windows and Linux keep today's flow. — lead, 2026-10-06 (plan review)

## 11. Clearing the orphan (dev Macs only, R6)

Not product code. It lives in a scratch directory and is deleted afterwards.

1. **Guus creates two Apple Development provisioning profiles**, for `io.beebeeb.desktop` and `io.beebeeb.desktop.FileProvider` (app group `R8352WDJJR.io.beebeeb.desktop.fileprovider`). He recreates the App IDs if May's are gone.
2. **Build a minimal helper app** `io.beebeeb.desktop` embedding an empty `io.beebeeb.desktop.FileProvider` extension, signed with those profiles. Its only action is `NSFileProviderManager.removeAllDomains`, printing the result.
3. **Run it once**, then delete the helper and the two profiles.
4. **Proven by four checks:**
   - the xattr and the `Beebeeb-Beebeeb` folder are gone
   - `fileproviderd` no longer writes domain properties for `io.beebeeb.desktop.FileProvider`
   - our add succeeds
   - `BeebeebFileProviderCtl status` prints `installed`

It runs **after** device check D0 has captured the real `folder_taken` code on this Mac. It is offered to Bram for his Mac if `ls ~/Library/CloudStorage` shows the same xattr there.

## 12. Design artefacts change first

Before the frontend change merges:
- `design/hifi/macos-menubar-popover.html`: the "Finder not added" artboard is removed, and the "Finder add failed" artboard shows the reason-keyed copy from 6.2.
- `docs/specs/2026-09-30-macos-menubar-popover.md`: an amendment line records R5 and points here.
- `design/hifi/macos-settings-dialogs.html`: the Repair dialog no longer says "You can add it back afterwards" (Repair adds Beebeeb back by itself), and the different-account warning (R8) is drawn with its exact copy. — lead, 2026-10-06 (plan review)

## 13. Testing and verification

### 13.1 Automated

Every new test is seen failing before it passes, and the failure output is pasted into the task Notes.

- **`core`**: table tests for every trigger × every observation (section 5); merged triggers (N triggers during one check → exactly 1 follow-up); a revoked session never emits remove; sign-out cancels and removes; Repair removes and then checks exactly once; `Missing` is never a resting state while wanted is present.
- **`policy`**: on an injected clock, transient failures make exactly 4 attempts at 0/5/15/45 s; `unknown` makes 2; not-retryable reasons make 1; no attempt after `Failed` without an explicit trigger. `classify` covers every row of 6.2 plus the fallback. It is **mutation-checked**: flip one mapping, confirm the matching row's test fails, revert.
- **Bridge**: a stub returns each (domain, code) pair, and Rust receives it unchanged.
- **`launch_location`**: `/Applications`, `~/Applications`, `/private/var/folders/…/AppTranslocation/…`, `/Volumes/Beebeeb/…`, `~/Downloads/…`.
- **`lifecycle_log`**:
  - rotation at 1 MB, keeping 3
  - a known file name planted inside an NSError message comes out as `[name]`, and a path as `[path]`
  - an ordinary `tracing::warn!` with a path never appears in the file
- **Config**: a 0.8.11 `desktop.toml` with the old keys loads.
- **Re-sign-in (R8)**, tests seen failing first and mutation-checked: a re-sign-in never purges the queue, never removes the domain and never touches the keys; a mismatch changes nothing local; every sign-in method (password, 2FA, browser) goes through the same account check; Windows keeps today's flow. — lead, 2026-10-06 (plan review)
- **Security review (R8):** the `crypto-security-reviewer` agent reviews the re-sign-in diff before it merges, and its findings go into the task Notes. — lead, 2026-10-06 (plan review)
- **Account binding (R10)**, tests seen failing first and mutation-checked:
  - a different account never reuses local data
  - the same account never loses any (R8)
  - local data without a recorded owner is reset first (fail closed)
  - a failed reset blocks the engine
  - a failed sign-out purge stops the sign-out on macOS and Linux
  - every engine start goes through the binding
  - Windows refuses, tested on CI's Windows job
  The same security review covers R9 and R10; its R10 findings also go to private task 1835. — lead, 2026-10-06 (plan review)
- **Every retained trace counts**, tests seen failing first and mutation-checked. After a revoked token, with nothing queued, on an install from before R10:
  - another account is a switch, never a first sign-in
  - after "Sign out and switch", no vault key or email of the first account remains
  - the same account signs in again in place and keeps its keys (R8, R9)
  — lead, 2026-10-06 (plan review)
- **Frontend (bun)**:
  - the Finder step advances on a `Ready` event
  - each reason renders exactly one notice and the action from 6.2
  - no "Install" button renders on macOS
  - a source-contract test that no macOS code path calls `install_finder_location`
- **Gates**:
  - `cargo test --locked` in `src-tauri`, with the per-binary `test result: ok. N passed; 0 failed` lines
  - ~~`cargo clippy --all-targets`~~
    No new warnings from `cargo clippy --locked --all-targets` compared with an `origin/main` baseline. CI runs no clippy, and the tree is not known to be clean under `-D warnings`. — lead, 2026-10-06 (plan review)
  - `bun test` with the `N pass / 0 fail` count
  - `bunx tsc --noEmit`
  - eslint

### 13.2 Device checks on Guus's Mac (R3)

Evidence for every check goes under `.claude/tasks/_qa-evidence/<task>/`:
- a screenshot
- the relevant `lifecycle.log` lines
- `BeebeebFileProviderCtl status` output
- the count of `fileproviderd` "Adding domain" lines for the attempt (`log show --predicate 'process == "fileproviderd" AND eventMessage CONTAINS "beebeeb"'`)

| # | Check | Pass means |
|---|---|---|
| D0 | Before the cleanup, the new build meets the orphan | `folder_taken` copy shown once; exactly 1 add attempt; ~~the NSError domain and code recorded into 6.2~~ the NSError domain and code confirmed against 6.2 (`NSCocoaErrorDomain` 516 expected) and the underlying error recorded; whether Console.app lists the lifecycle log recorded (— lead, 2026-10-06 (plan review)) |
| D1 | Orphan cleanup (section 11) | all four checks |
| D2 | Signed out, then sign in | Beebeeb in Finder within 30 s with zero clicks after sign-in; a cloud-only file opens |
| D3 | Quit and relaunch | `Ready`; nothing asked; at most 1 add attempt |
| D4 | Sign out, then sign in | `status` `missing` and the sidebar entry gone; then back by itself |
| D4b | "Repair…" in Settings while signed in | Beebeeb leaves Finder and comes back by itself; ≤4 add attempts |
| D5 | ~~Session revoked from the web~~ Session revoked from the web, then "Sign in again" as the **same** account (R8) | ~~Finder stays; sign-in requested; an edit made meanwhile reaches the server after sign-in (version visible on the web)~~ Finder stays; sign-in requested; the re-sign-in replaces only the token (no recovery phrase, no sign-out); the queue is not purged; an edit made meanwhile reaches the server afterwards (version visible on the web). Repeated after a relaunch while revoked (— lead, 2026-10-06 (plan review)) |
| D5b | Session revoked, then sign in as a **different** account (R8) (— lead, 2026-10-06 (plan review)) | the warning names the pending-edit count; "Cancel" changes nothing; "Sign out and switch" does the full sign-out (domain removed, queue purged), then the new account signs in fresh |
| D6 | Turned off in System Settings, then back on | the `user_disabled` notice; then `Ready` with no click in Beebeeb; ~~exactly 1 add attempt after the flip~~ at most 1 add attempt after the flip: turning it back on leaves the domain registered, so §5.5 step 3 only confirms it (— lead, 2026-10-06 (plan review)) |
| D7 | Opened from the mounted dmg | `not_in_applications` reported by `finder_setup_state` before sign-in |
| D9 | Account binding (R10), with two local test accounts: the device check in private task 1835, first on a build of `origin/main`, then on the QA build (— lead, 2026-10-06 (plan review)) | as recorded in private task 1835; its evidence stays in the private workspace |
| D8 | Update 0.8.11 → the alpha carrying this spec, while signed in | `Ready` after the relaunch; nothing asked; ≤4 add attempts |

**Environment:**
- D0–D7 run a build signed with the existing `io.beebeeb.app` development profiles (valid to May 2027), against the local API (`BB_API_BASE=http://localhost:3001`), with a test account.
- **D8 runs against production** (R7): the published 0.8.11 dmg, then the real alpha update, with a test account.
- Guus's own session is signed out during the run. Restoring it (his password) is the last step.

**What counts as done:**
- **Done** = every automated gate green with counts, plus D0–D9 passed with evidence (D9 added for R10, — lead, 2026-10-06 (plan review)).
- A check that cannot run gets its line amended in place, per `.claude/tasks/README.md`. It is never satisfied badly.

## 14. Docs that change with the code

- `repos/desktop/CLAUDE.md` → "Current macOS integration state": the reconciler, the log path, and the 1698 correction.
- `docs/CAPABILITIES.md`, if it describes Finder installation.
- `RELEASE_NOTES.md` for the release that carries A, with counted test results (the release script refuses notes without them).

## 15. Open items, each with an owner

- ~~The `folder_taken` NSError domain and code: captured by D0, owner lead.~~
  The `folder_taken` NSError domain and code: very likely `NSCocoaErrorDomain` 516 (see 6.2); D0 confirms it, owner lead. — lead, 2026-10-06 (plan review)
- Whether the May App IDs `io.beebeeb.desktop` and `io.beebeeb.desktop.FileProvider` still exist in the developer portal: Guus, at step 1 of section 11.
- ~~The exact provisioning error codes mapped to `signing`: listed in the plan from the bridge's existing provisioning paths, owner lead.~~
  The `signing` codes: none can be derived from code. The set ships empty, and an optional device rung (D-S, a profile-less ad-hoc build) may capture one. Owner lead. — lead, 2026-10-06 (plan review)
- R8 keeps the keys on this Mac after a revoked session, including across a relaunch. Today a startup 401 deletes them, so revoking a lost device from the web effectively wipes it at its next launch; after R8 it does not. The security review and Guus confirm this trade-off. — lead, 2026-10-06 (plan review)
  Ruled R9 (§3): the key stays, and the security review decides. — lead, 2026-10-06 (plan review)
- ~~Identity for installs that predate `signed_in_user_id`: the plan falls back to the account email recorded on this Mac (case-insensitive), and an unknown identity counts as a different account (fail closed). The security review confirms this.~~ — lead, 2026-10-06 (plan review)
  Identity for installs from before R10: their local data is adopted at startup by the account email this Mac's Keychain still holds. Elsewhere R8 falls back to the recorded email (case-insensitive). An identity that cannot be compared is never treated as the same account. The security review confirms this. — lead, 2026-10-06 (plan review)
- ~~Windows and R10: the binding refuses instead of resetting, and Windows sign-out refuses while unsent changes exist. A Windows PC that holds another account's unsent changes therefore has no in-app way forward. Owner Guus (decision).~~ — lead, 2026-10-06 (plan review)
  Ruled R11 (§3): Windows stays fail-closed in this spec. The "Discard and switch" escape hatch is private workspace task 1837. — lead, 2026-10-06 (plan review)
- An app on an external disk's Applications folder (`/Volumes/<disk>/Applications`) is refused by the 6.2 rule. Is that intended? Owner Guus. — lead, 2026-10-06 (plan review)
