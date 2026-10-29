# macOS Finder Setup Reconciler Implementation Plan (spec A of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On a Mac, once someone is signed in and the Mac has their keys, Beebeeb adds itself to Finder with no click, stays there across launches, updates and revoked sessions (signing in again as the same account keeps Finder, keys, cache and pending edits: ruling R8), leaves only on a sign-out the person chose, and, when macOS blocks it, says one sentence with one action, stops retrying, and writes the reason to a log file.

**Architecture:** One Rust reconciler (`src-tauri/src/finder_setup/`) has four parts: a pure state machine (`core`), a pure retry and classification policy (`policy`), a pure launch-location check (`launch_location`), and a single async driver task (`driver`). The driver runs one `NSFileProviderManager` operation at a time through the ObjC bridge, which now returns structured `(domain, code, message, underlying)` errors. Every surface reads the published `FinderSetupView` (`finder_setup_state` command plus the `finder-setup-changed` event). On macOS, no surface installs anything any more. A small self-written `lifecycle_log` file records typed events with redacted error text. Ruling R8 adds re-sign-in in place: every sign-in method first checks which account is signing in (`reauth::sign_in_kind`). The same account swaps only its session token. A different (or unidentifiable) account is an account switch: a warning with the pending-change count, then the full sign-out, then a fresh sign-in. Ruling R10 binds the local data to the account that created it: every engine start checks the owner recorded in `state.db` first (`spawn_bound_engine`), and a different account never reuses that data.

**Tech Stack:** Rust 2024 (Tauri v2, tokio `sync`/`time`/`rt`, serde, chrono, sysinfo, no new crates), Objective-C (`FileProvider.framework`, compiled by `src-tauri/build.rs`), React 19 + TypeScript (bun test with `tests/fixtures/componentHarness.ts`), eslint.

**Spec:** `docs/specs/2026-10-06-macos-finder-setup-reconciler.md` (approved by Guus 2026-10-06). Executors read the spec and this plan together. Where the two disagree, see "Spec issues found" directly below. Nothing there is silently decided. The spec was amended in place on 2026-10-06 for ruling R8 and for this plan's review (its **Amended** line). Struck originals stay readable.

---

## Spec issues found

Each item has file:line evidence from this worktree (`c351f98`) or from Guus's Mac.

**Round 2 (2026-10-06, after Guus's ruling R8).** The spec is now amended in place for items 1–10 and 12–16. The originals are struck through and the signed replacements sit beneath them (see the spec's **Amended** line). Item 11 stays an open question in spec §15. The interface refinements in Tasks 2, 3 and 7 (the `real_home` parameter, the `step`/`next_op` split, the typed handle methods) are plan-only. Items 17–21 are new in round 2.

**Round 3 (2026-10-06, rulings R9 and R10).** R9 answers item 18. R10 folds the binding of local data to its account into this plan as Task 10, the old items 17 and 19 are rewritten, and items 22–23 are new. The tasks are renumbered (20 → 21). The spec records R9, R10 and §5.6.

**Round 4 (2026-10-06, Codex review of PR #113 and ruling R11).** Spec issue 24 is a P1 that Codex found. It is fixed in Tasks 11 and 12, with tests that reproduce it first. R11 rules the Windows gap of issue 17.

~~**Item 1 needs a ruling from Guus before device check D5 can pass.** Items 6, 11 and 16 are smaller questions, queued in Task 21 step 1. Every other item is resolved in this plan as stated, and the reviewer can reject any one of them on its own.~~ (Superseded by round 2, above.)

1. **R2 and D5 contradict the existing re-sign-in path ~~(needs Guus)~~ (answered by R8).** After a session is revoked, the only way back in is `AuthExpiredBanner` → `forceReauth` (`src/desktopApi.ts:1030-1034`), which calls `clear_session` and then opens onboarding. `clear_session` *is* the "sign-out by choice" of spec §5.3(5), so it removes the domain. It also purges the operation queue and the decrypted cache (`src-tauri/src/lib.rs:1802-1844`, the task 1538 cross-account control). Spec R2 says "files stay listed … edits made meanwhile are kept and upload after sign-in". D5 says "Finder stays; … an edit made meanwhile reaches the server after sign-in". Neither holds while re-auth goes through `clear_session`. ~~**This plan does not change `forceReauth` or the purge.** That is security-sensitive and belongs to spec C (sign-in flow). The plan implements R2 everywhere the reconciler decides (see item 2), and D5 runs and reports the true result. The decision goes to `.claude/tasks/decisions/` (Task 21, step 1). Candidate answer for Guus: a `reauth` path that keeps the domain and, only for the *same* account id, keeps the queue.~~ **Resolved 2026-10-06 by ruling R8** (re-sign-in in place, spec §3). Tasks 11, 12 and 18 implement it, and device checks D5/D5b check it.
2. **A startup 401 signs the app out without the person choosing it.** `restore_session_on_startup` → `discard_unusable_startup_session` (`lib.rs:599-622`) clears the keychain and boots signed out when the stored token is revoked. If "wanted" came from "is anyone signed in", the reconciler would remove the domain on that launch, which breaks R2. **Resolution in this plan:** the persisted intent `finder_signed_out_by_choice` (Task 6) is set only by a real sign-out and cleared at the next sign-in. Signed out *and* that flag set means `absent`. Signed out without the flag means `no action`. Core test `every_trigger_against_every_observation…` pins the 401 row.
3. **The keys-arrive list in spec §5.3(2) is not the code's.** `desktop_login` and `desktop_login_2fa` never put a master key in memory. They store only the token (`lib.rs:1100-1118`, `1225-1245`). Keys arrive in exactly four functions: `apply_session` (browser handoff, `lib.rs:1541`), `desktop_unlock_with_recovery_phrase` (`lib.rs:1267`, two branches), `unlock_vault` (`lib.rs:1936`, two branches; missing from the spec) and `restore_session_on_startup` (`lib.rs:657`; covered by the launch trigger, which this plan fires *after* the restore finishes so the check sees the restored keys). Tasks 8 (launch) and 9 (the other three) wire all four, and a source-contract test proves that no other function installs a `Session`, including the two login commands. Round 2: R8's `reauth_in_place` (Task 11) is a fifth path. It also calls `keys_arrived`, and the Task 9 census counts it.
4. **The lifecycle log cannot be at the real `~/Library/Logs` under the app sandbox.** The shipped app is sandboxed (`src-tauri/entitlements.plist`: `com.apple.security.app-sandbox`). Guus's live config is at `~/Library/Containers/io.beebeeb.app/Data/Library/Application Support/beebeeb/desktop.toml`. So `~` resolves to the container, and the file is `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log`. Writing to the real `~/Library/Logs` would need a temporary-exception entitlement, which is a provisioning change. Whether Console.app lists the container's log is **unverified**. D0 checks it and records the answer, and the docs (Task 19) state the container path.
5. **The `folder_taken` code is very likely already known.** Guus's 0.8.11 app saved the collision in its config at 15:05 on 2026-10-06 (`finder_install_last_attempt_at = 1791291909` = 13:05:09Z, inside the 15:04:31–15:05:09 local failure window): `finder_install_last_error = "The file couldn't be saved because a file with the same name already exists. (NSCocoaErrorDomain 516)"`. That is `NSFileWriteFileExistsError`. Task 1 therefore maps top-level `(NSCocoaErrorDomain, 516)`, or an underlying `(NSPOSIXErrorDomain, 17)` EEXIST, to `folder_taken`, marked provisional. D0 still has to confirm the top-level pair and capture the underlying error before the PR merges. If D0 shows anything else, the row is amended first (Task 21 step 5, row D0). This also removes the ordering knot in spec §6.2/§13.2, where D0 needs the row and the row needs D0.
6. **`signing` has no derivable codes.** `FileProviderBridge.m` has no provisioning-specific path: it passes NSErrors through and synthesizes four messages (lines 50, 66, 155, 173). The old classifier matched English substrings (`lib.rs:2112`). `policy::SIGNING_CODES` therefore starts **empty**, and a test pins it as empty, so the gap is visible and not silent. A signing failure falls back to `unknown` (one retry, then "Beebeeb couldn’t be added to Finder." with Try again). Capturing a real code is the optional rung D-S in Task 21 step 5.
7. **Engine start order.** Spec §5.5 step 5 says "On Ready, start the sync engine". The code that step names (`start_engine_for_pending_finder_install`, `lib.rs:2568`) starts the engine *before* `addDomain` and waits for the daemon socket, because the extension talks to the daemon (`lib.rs:2614-2626`). The plan keeps that order: `StartEngine` comes before `AddDomain`, the sync folder is saved on `Ready`, and an engine the check started is stopped when the check ends in `Failed` or `UserDisabled`. That is the old `stop_pending_finder_install_engine` behaviour.
8. **Lock is a race the spec does not list.** Now that the reconciler starts the engine by itself, a check that is running while `lock_vault` stops the engine could start a new one after the stop (`lock_vault`, `lib.rs:2002`). The plan adds `Trigger::Lock`: it cancels the check, holds, and is acknowledged before `lock_vault` stops the engine (Task 9). Sign-out gets the same ordering. The reconciler's removal now runs at the **start** of `clear_session_impl`, before the engine stop, where it used to run after the purge.
9. **"Show in Finder" needs a command.** Spec §9 lists four commands, but the `not_in_applications` action reveals the app bundle. The plan adds `finder_setup_show_app` (Rust `reveal_item_in_dir` of the running bundle, Task 8).
10. **-2005 and -2013 are in the lead's verified code list but in no §6.2 row.** They classify as `unknown` (one silent retry), and a test pins that.
11. **`/Volumes/…/Applications/Beebeeb.app` is refused.** That is §6.2 taken literally ("starting with /Volumes/ → not OK"). An app installed on an external disk's Applications folder is told to move. The rule is pinned by a test and listed in Review Focus for Guus.
12. **Repair copy contradicts R5 and §5.1.** `REPAIR_BODY` (`src/macSettingsModel.ts:145`) and `design/hifi/macos-settings-dialogs.html:210,216,327` say "You can add it back afterwards." After this spec, Repair re-adds by itself. The artefact changes first (Task 13), then the code (Task 16). New body: "Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept."
13. **CI runs no clippy.** `.github/workflows/ci.yml` has `cargo check --locked --all-targets` and `cargo test --locked` but no clippy step, and whether the tree is clean under `-D warnings` is unknown. The gate is therefore **no new clippy warnings** compared with an `origin/main` baseline, plus zero warnings in files this plan creates (Task 1 step 0, Task 21 step 2).
14. **A macOS surface the spec does not list.** `src/windows/views/SettingsView.tsx:873` makes the macOS main window's Finder panel call `install_finder_location`. Task 17 moves it onto `finder_setup_state`.
15. **Apostrophes.** Spec §6.2 writes straight quotes ("hasn't"). Product copy in this repo uses the typographic `’` (for example `src/macSettingsModel.ts:91`). `finderSetupCopy.ts` and the artefact use `’`, and the copy test pins those exact strings.
16. **D6's "exactly 1 add attempt after the flip" vs §5.5 step 3.** Turning the extension back on leaves the domain registered. §5.5 step 3 then only confirms stability, so the app itself calls `addDomain` **0** times after the flip (core test `user_disabled_poll_reads_only_while_visible_and_a_flip_runs_one_check`). fileproviderd may still log its own re-registration. D6 is therefore gated as "≤ 1" (Task 21), and the wording goes to Guus.
17. **Local data is bound to the account that created it (ruling R10, hardening).** `state.db` and the files it points at carried no record of their account. Background: the private workspace task, which stays out of this public repo. Task 10 adds the binding: an owner record inside `state.db`, checked before every engine start; a different or unrecorded owner's data is reset first; a failed reset or sign-out purge stops. **Windows is covered, but it refuses where macOS and Linux reset.** A Windows PC that holds another account's unsent changes cannot start sync for the new account. The Windows sign-out, which refuses while unsent changes exist (`windows_cf/signout.rs:24`), cannot clear it either, and Windows sign-in refuses while a session is present. That leaves no in-app way forward, ~~which is a decision for Guus (Task 21 step 1)~~. **Ruled R11 (Guus, 2026-10-06):** "Refuse now, escape hatch as its own task". Windows stays fail-closed in this plan, and Task 10's Windows tests prove the refusal. The explicit "Discard and switch" escape hatch is private workspace task 1837.
18. **R8 keeps the keys after a remote revocation.** Today a startup 401 deletes the vault key (`lib.rs:599-622`), so revoking a lost device from the web wipes its keys at its next launch. Task 12 keeps the key so the same account signs in again without its recovery phrase (R8: "keys … all stay"). That is a security trade-off. It is question 5 of the mandatory review (Task 11 step 9), and Task 12 merges only with that review's OK. Its fallback is described in Task 12. **Answered 2026-10-06 by ruling R9:** "Keep the key, review decides". The key stays. Task 12 merges only if the security review agrees, and if it disagrees the decision returns to Guus.
19. ~~**Identity for installs from before R8.** No server user id is stored today. Task 6 adds `signed_in_user_id`: written at sign-in, backfilled by the startup probe (Task 12) while the session still works, and cleared by a sign-out by choice. Without it, the account check falls back to the account email recorded on this Mac (case-insensitive), and an unknown identity counts as a different account (fail closed). This is review question 1.~~
   Round 3: the identity is the owner record of the local data, inside `state.db` (Task 10), not a `desktop.toml` key. Local data from before R10 is adopted at startup by the email this computer's Keychain still holds. R8 falls back to the owner's email, then the Keychain email, then `last_signed_in_email` (case-insensitive). An identity that cannot be compared is never the same account. This is review question 1.
20. **A mismatch revokes the newly minted session.** After "Sign out and switch" the person enters the other account's password again. That costs one extra sign-in, but leaves no second live session on the Mac and no new token held across a purge.
21. **The switch warning is new UI copy.** Task 13 draws it in `design/hifi/macos-settings-dialogs.html` before any code (design first). Guus reviews the wording in that PR.
22. **The private task's fix boundary names one function, but engines start in five places.** They are `start_engine_if_possible` (`lib.rs:851`), `persist_sync_root_and_start_engine` (`lib.rs:2562`), `start_engine_for_pending_finder_install` (`lib.rs:2612`), `pick_sync_root` (`lib.rs:4909`) and Task 8's `ensure_sync_root_and_engine`. Onboarding's folder pick and the Finder reconciler bypass the first. So Task 10 binds at the spawn (`spawn_bound_engine`, the only remaining `EngineRunner::spawn(`), and a source test pins it. It also puts the owner inside `state.db` rather than a separate file next to it, with the evidence given in Task 10.
23. **A failed sign-out now stops on macOS and Linux (R10).** The engine has already stopped and Beebeeb has already left Finder (Task 9 removes it first). ~~So a person whose sign-out fails is still signed in, with no sync and no Finder entry, until they try again or relaunch.~~ The error says to try again. Windows already behaves this way (`lib.rs:1751`).
    Superseded during execution (spec §5.6): a sign-out that stops this way while the keys are still in memory leaves the person signed in, and the reconciler is told the keys are here, so Finder comes back and its check starts the engine again (same account, owner record untouched). Only a sign-out that stops while an earlier engine stop is unconfirmed does not ask for Finder back, because every start refuses until Beebeeb restarts. — lead, 2026-10-08 (final review)
24. **The account check missed traces that a startup 401 keeps (Codex P1 on PR #113, confirmed).** Round 2's `traces` (Task 11 `settle_sign_in`) was:
    - the session in memory
    - `auth_present`
    - `keychain_session_present`, which reads only the token (`lib.rs:443-458`)
    - pending changes

    Task 12 drops only the token and keeps the vault key and email. Task 10 can adopt an email-only owner. So on an install from before R10, with a revoked token and nothing queued, `traces` was false. `sign_in_kind` then returned `Fresh` before it compared the email (`if !local.traces`, Task 11 step 2). A different account would then have signed in next to the previous account's vault key, and a later Keychain restore would have loaded that key with the new account's token.

    **Fixed:**
    - `reauth::LocalTraces` counts every retained trace: session, `auth_present`, token, Keychain email, vault key, recorded owner, cached profile, queued or staged data. `Fresh` needs none of them.
    - A different or unknown account takes the switch, and a mismatch still changes nothing locally (Task 11).
    - The switch's sign-out stops unless the Keychain holds no vault key and no email afterwards (Task 12).
    - Windows is unaffected: its startup 401 still clears everything, and `settle_sign_in` does not run there.

## Global Constraints

- macOS only. Windows CFAPI and Linux FUSE behaviour stay unchanged. `install_finder_location`, `continue_without_finder_location`, `finder_location_state` and `finder_domain_user_enabled` remain registered and keep working on Windows/Linux (spec §4, §9). On macOS no code path calls them.
- No new dependency: `Cargo.lock` and `bun.lock` are unchanged at the end (spec §8: "our own small writer, with no new dependency"). `git diff origin/main -- src-tauri/Cargo.lock bun.lock` is empty.
- One closed vocabulary: `FinderSetup {Ready, Missing, Adding, Failed, UserDisabled}` (existing, `surfaces/phase.rs`) and `FinderFailureReason {extension_loading, user_disabled, not_in_applications, folder_taken, signing, timeout, unknown}`, serialized `snake_case`.
- Retry: transient (`extension_loading`, `timeout`) at most **4 attempts at 0 s, 5 s, 15 s, 45 s** after the check starts, with a late attempt delaying the next start. `unknown`: **2 attempts** (0 s, 5 s). Not retryable (`folder_taken`, `signing`, `not_in_applications`): **1**. After `Failed`, nothing retries until launch, keys arriving, or "Try again". `UserDisabled` poll: read-only, **every 3 s while a Beebeeb window is visible, plus once when the app becomes active**. Stabilization wait after add: **≤ 10 s**.
- Classification reads only NSError domain + code (and the `NSUnderlyingErrorKey` error's domain + code). The message goes only to the lifecycle log, redacted. No substring matching on macOS.
- One sentence and one action per failure. A failure that gates Finder is inline, never a toast (1683 ruling 7). A failed *action* (a button that gates nothing) is a toast.
- All Finder-setup copy lives in `src/finderSetupCopy.ts`. The exact strings (typographic `’`, see Spec issue 15):
  - Adding: `Adding Beebeeb to Finder…`
  - extension_loading: `macOS hasn’t finished loading Beebeeb’s Finder extension.` → **Try again**
  - user_disabled: `Beebeeb is turned off in System Settings.` → **Open System Settings**
  - not_in_applications: `Beebeeb is running from the disk image. Move it to Applications, then open it again.` → **Show in Finder**
  - folder_taken: `An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.` → **Copy details**
  - signing: `This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.` → **Copy details**
  - timeout: `macOS didn’t finish adding Beebeeb to Finder.` → **Try again**
  - unknown: `Beebeeb couldn’t be added to Finder.` → **Try again**
- No "Add to Finder" and no "Install … Finder" button anywhere on macOS (R5). "Try again" appears only inside a failure.
- "Copy details" copies: app version, macOS version, reason, NSError domain + code, attempt count, time of the last attempt, plus the last 200 lifecycle-log lines. It never includes paths, file names, the email or the account id.
- Lifecycle log: `<home>/Library/Logs/Beebeeb/lifecycle.log`, where `<home>` is the container under the sandbox (Spec issue 4). It rotates at **1 MB** and keeps **3** files (`lifecycle.log`, `.1`, `.2`). It is not a `tracing` sink and it survives sign-out.
- `desktop.toml`: the old `finder_install_*` keys still load, are ignored on macOS, and are no longer written there. New keys: `finder_last_failure = { reason, domain, code, at }` and `finder_signed_out_by_choice`. The account identity lives in `state.db` (R10, Task 10), not here.
- Brand: no emoji, the amber accent only for primary actions and encryption state, Inter/JetBrains Mono as already used. "Europe"/"the EU" wording is untouched.
- Design before code: the artefact changes (Task 13) land on the UI branch **before** any frontend commit (spec §12).
- Evidence: every new test is seen failing first, and the failure is pasted into the task file's Notes. The tests the spec names are mutation-checked. Each task reports counts (`test result: ok. N passed; 0 failed`, `N pass / 0 fail`), never adjectives.
- Process: one worktree per lane. `CARGO_TARGET_DIR` is never shared between worktrees. Heavy cargo runs go through `/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/scripts/coord/with-lock.sh cargo-build -- …` (rc 75 = busy, retry). No `git stash`, no `pkill`, foreground commands only. Lanes do not commit `graphify-out/`. Commits use an explicit pathspec and end with the trailer of the model that wrote them (for example `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`).
- **R8, re-sign-in in place.** A same-account sign-in replaces only the session token (Keychain and memory). It never purges the queue, never clears or rewrites keys, never removes the Finder domain, and never signs out. A different account, or one whose identity cannot be established, is an account switch: the warning with the pending-change count, then the full sign-out, then a fresh sign-in. On a mismatch nothing local changes. Windows keeps today's sign-in refusal and today's "Sign in again" flow.
- **R8/R9/R10, security.** The `crypto-security-reviewer` agent reviews Tasks 10–12 before Lane R's PR merges (Task 11 step 9). Its findings are recorded verbatim in the task Notes, and the R10 findings also in the private workspace task's Notes.
- **R10, local data belongs to one account.** Every engine start binds first (`spawn_bound_engine`, the only `EngineRunner::spawn(` in production). Another account's or an unrecorded owner's data is reset before anything starts (Windows refuses instead). An owner that cannot be compared stops the start and deletes nothing. A failed reset or sign-out purge stops. Public wording stays neutral ("local data is bound to the account that created it; a different account never reuses it"); background lives only in the private task.

## Review Focus

These are the five conditions most likely to hurt a real person that the spec implies but no test in its list exercises. Each line names the test that now pins it and the task that owns that test.

1. **Relaunch after a revoked session** (the startup 401 discards the token): a person expects Beebeeb to stay in Finder and ask for sign-in (R2), not to vanish. Pinned by core test `every_trigger_against_every_observation_ends_in_a_state_a_person_can_act_on` (row "revoked at startup (401), registered": zero `RemoveDomain`), Task 3.
2. **Signing out while a check is mid-flight** (for example 8 s into a 10 s stabilization wait): a person expects sign-out to finish with Beebeeb gone from Finder and no engine left running with their keys. Pinned by driver test `an_event_sent_during_an_operation_waits_for_it_and_wins_over_the_rest_of_the_check` (Task 7), source test `sign_out_lock_and_repair_reach_the_reconciler_before_the_engine_stops` (Task 9), and behavioural test `sign_out_waits_for_the_reconciler_removal` (Task 9).
3. **Locking the vault while a check runs**: a person expects no engine afterwards. Pinned by core test `lock_cancels_the_check_and_holds_until_keys_arrive` (Task 3) and the Task 9 source test.
4. **A Beebeeb window left open while the extension is off in System Settings**: a person expects a cheap read-only poll that never calls add, and exactly one check when they flip it back on. Pinned by core test `user_disabled_poll_reads_only_while_visible_and_a_flip_runs_one_check` (Task 3) and D6.
5. **An app on an external disk's Applications folder** (`/Volumes/Data/Applications/Beebeeb.app`): a reasonable person expects it to work. ~~The spec says refuse. Pinned *as the spec rules* by `volumes_is_refused_even_under_an_applications_folder` (Task 2), and listed as Spec issue 11 for Guus to confirm.~~
   Corrected during execution (spec §6.2): only a bundle at a volume's root is a disk image, so an Applications folder on another disk is a real install and is not refused. The test that pins it is `an_applications_folder_on_another_disk_is_a_real_install` (Task 2); the old test name no longer exists. Spec issue 11 is answered the same way. — lead, 2026-10-08 (final review)

---

## File map

Paths are relative to the desktop repo root (the worktree).

| File | Status | Responsibility | Task |
|---|---|---|---|
| `src-tauri/src/surfaces/phase.rs` | modify | add `FinderFailureReason`, `FinderSetup::as_str`, `PopoverSnapshot.finder_reason` | 1, 8 |
| `src-tauri/src/finder_setup/mod.rs` | create | module root and docs | 1 (grows in 2, 3, 7, 8) |
| `src-tauri/src/finder_setup/error.rs` | create | `FpError`, `FpErrorCode`, domains, codes, `FailureRecord` | 1 |
| `src-tauri/src/finder_setup/policy.rs` | create | `classify`, `RetryPolicy`, SDK code constants | 1 |
| `src-tauri/src/finder_setup/launch_location.rs` | create | `LaunchLocation`, `launch_location`, `bundle_path_from_exe`, `current` | 2 |
| `src-tauri/src/finder_setup/core.rs` | create | pure state machine: `step`, `next_op`, all decisions of spec §5 | 3 |
| `src-tauri/macos/FileProviderBridge.m` | modify | `BeebeebFpError` struct, structured errors, test fill hooks | 4, 8 |
| `src-tauri/src/macos_file_provider.rs` | modify | `BeebeebFpErrorC` mirror, `Result<_, FpError>` API, 1698 correction | 4, 8 |
| `src-tauri/src/lifecycle_log.rs` | create | the lifecycle log writer, typed events, redaction | 5 |
| `src-tauri/src/state_db.rs` | modify | `StateDb::known_names`; R10: the owner record, `ACCOUNT_TABLES`, `has_account_data`, `clear_account_data`, the purge also clears staged payloads and the owner | 5, 10 |
| `src-tauri/src/config.rs` | modify | `finder_last_failure`, `finder_signed_out_by_choice` | 6 |
| `src-tauri/tests/fixtures/desktop-0.8.11.toml` | create | a 0.8.11 config with the old keys | 6 |
| `src-tauri/src/finder_setup/driver.rs` | create | the one reconciler task, `FinderSetupHandle`, `FinderSetupView`, `copy_details_text` | 7 |
| `src-tauri/src/finder_setup/macos_ports.rs` | create | the macOS `Ports`: bridge, engine, emit, log, config | 8 |
| `src-tauri/src/lib.rs` | modify | `AppState.finder_setup`, commands, setup spawn, popover, legacy cfg-split, trigger wiring; R10: `spawn_bound_engine` and its five callers, `StateDbLocalData`, the startup adoption, the stopping sign-out purge; R8: `LoginOutcome`, `settle_sign_in`, `reauth_in_place`, `open_reauth_window`, the startup-401 split, the probed profile | 8, 9, 10, 11, 12 |
| `src-tauri/src/popover_data.rs` | modify | drop `finder_adding`; reason is `FinderFailureReason` | 8 |
| `src-tauri/src/account_binding.rs` | create | R10: the pure binding decision and its executor | 10 |
| `src-tauri/src/reauth.rs` | create | R8: the pure account check `sign_in_kind` | 11 |
| `src-tauri/src/browser_login.rs` | modify | R8: the browser handoff goes through the same account check | 11 |
| `src-tauri/src/keychain.rs` | modify | R8/R9: `AuthVault::clear_session_token` (token only) | 12 |
| `src-tauri/src/account_dto.rs` | modify | `AccountProfile` derives `PartialEq, Eq` (the probe returns it) | 12 |
| `design/hifi/macos-menubar-popover.html` | modify | remove "Finder not added"; reason-keyed "Finder add failed" | 13 |
| `design/hifi/macos-settings-dialogs.html` | modify | Repair copy; the R8 account-switch warning | 13 |
| `docs/specs/2026-09-30-macos-menubar-popover.md` | modify | R5 amendment line | 13 |
| `src/finderSetup.ts` | create | types, `loadFinderSetup`, `subscribeFinderSetup`, actions | 14 |
| `src/finderSetupCopy.ts` | create | every Finder-setup string, `finderSetupPresentation` | 14 |
| `src/Onboarding.tsx` | modify | `MacFinderStep`, macOS routing; R8: `mode`, the outcome routing, `AccountSwitchStep` | 15, 18 |
| `src/MacSettings.tsx`, `src/macSettingsModel.ts` | modify | Sync tab Finder row on the view; Repair copy | 16 |
| `tests/render-mac-settings.mjs` | modify | fixtures use `finder_setup_state` | 16 |
| `src/pages/SyncFolder.tsx`, `src/pages/Status.tsx` | modify | macOS reads the view and subscribes | 17 |
| `src/windows/views/SettingsView.tsx` | modify | `MacFinderIntegrationPanel` + dispatcher | 17 |
| `src/finderInstallCard.ts` | modify | delete the macOS-only helpers | 17 |
| `src/desktopApi.ts`, `src/onboardingSignIn.ts`, `src/main.tsx` | modify | R8: `forceReauth` in place on macOS, the sign-in outcome, the `mode` param | 18 |
| `src/accountSwitchCopy.ts` | create | R8: the switch warning's copy | 18 |
| `tests/finderSetupCopy.test.ts`, `tests/onboardingFinderStep.test.tsx`, `tests/statusFinderSetup.test.tsx`, `tests/finderSetupSourceContract.test.ts`, `tests/reauthInPlace.test.tsx` | create | frontend tests | 14–18 |
| `tests/macSettingsTabs.test.tsx`, `tests/macSettingsModel.test.ts`, `tests/finderInstallOneSurface.test.tsx`, `tests/finderInstallCard.test.ts`, `tests/forceReauth.test.ts`, `tests/onboardingSignIn.test.ts` | modify | follow the new contract; Linux/Windows rows kept as proof | 16, 17, 18 |
| `CLAUDE.md`, `docs/CAPABILITIES.md` | modify | docs (spec §14) | 19 |
| `$SCRATCH/orphan-cleaner/*` | scratch only | the one-off helper (spec §11), never committed | 20 |

`src-tauri/src/runner.rs` needs no change: it logs `error = %e`, and `FpError` implements `Display` (Task 4).

## Lanes, branches and order

- **Lane R (Rust)**: Tasks 1–12, **sequential**, one worktree, one branch `feat/$TASK-finder-reconciler`. Each task commits on that branch. Every task depends on the types from the one before.
- **Lane T (design + frontend + docs)**: Tasks 13–19, **sequential**, one worktree, branch `feat/$TASK-finder-reconciler-ui`. Task 13 (design) must be the first commit on this branch. Lane T only needs the type contract from Tasks 1, 2, 7 and 11 (`FinderSetupView`, the reason strings, the launch-location strings, and R8's `LoginOutcome` and `open_reauth_window`). It is spelled out in Tasks 14 and 18, so Lane T runs **in parallel** with Lane R. Its tests mock the commands.
- **Merge order**: Lane R's PR merges first, and only after the mandatory `crypto-security-reviewer` run on Tasks 10–12 (Task 11 step 9) has its findings recorded in the task Notes. Lane T then rebases onto `origin/main`, re-runs its gates, and its PR merges. Tasks 20 and 21 are **lead-only** and run on a build of Lane T's head *before* Lane T's PR merges (D0–D7). D8 runs after the alpha is published.
- **Worktree creation** (lead, from the workspace root; `$TASK` is the id the lead allocates from `.claude/tasks/_next-id`):

```bash
export TASK=<allocated id>
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop fetch origin
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add ~/code/bb-worktrees/desktop-$TASK-r -b feat/$TASK-finder-reconciler origin/main
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add ~/code/bb-worktrees/desktop-$TASK-t -b feat/$TASK-finder-reconciler-ui origin/main
```

Each lane sets `WT` to its own worktree and never builds in the other one. Cargo uses the worktree's own `src-tauri/target`, so `CARGO_TARGET_DIR` stays **unset**. `LOCK=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/scripts/coord/with-lock.sh`. The evidence directory is `EVID=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/tasks/_qa-evidence/$TASK`. Lanes write evidence and Notes there, and the lead commits them (the workspace `.git` is lead-only).

The lane's loop command, used in every Rust task (from `$WT/src-tauri`):

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib <filter> > $EVID/<task>-<step>.log 2>&1; echo "rc=$?"; grep -E "test result:|error\[|FAILED|panicked" $EVID/<task>-<step>.log | head -20
```

If `rc=75`, the lock was busy: rerun. Before every Rust commit, also run `$LOCK cargo-build -- cargo check --locked --all-targets` (this is what CI runs on macOS) and expect `rc=0`.

---

## Task 1: Reason vocabulary, `FpError`, classifier and retry policy (pure)

**Lane R.** No OS, no clock. The tests compile and run on every platform.

**Files:**
- Modify: `src-tauri/src/surfaces/phase.rs` (after the `FinderSetup` enum, line ~40; tests at the end of `mod tests`)
- Create: `src-tauri/src/finder_setup/mod.rs`
- Create: `src-tauri/src/finder_setup/error.rs`
- Create: `src-tauri/src/finder_setup/policy.rs`
- Modify: `src-tauri/src/lib.rs` (module list, next to `mod engine_status;` at line ~31)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `crate::surfaces::phase::FinderFailureReason` (Copy, Serialize/Deserialize snake_case): `ExtensionLoading, UserDisabled, NotInApplications, FolderTaken, Signing, Timeout, Unknown`; `FinderFailureReason::ALL: [Self; 7]`; `fn as_str(self) -> &'static str`.
  - `FinderSetup::as_str(self) -> &'static str`.
  - `crate::finder_setup::error::{FpError, FpErrorCode, FailureRecord}` and the constants `FILE_PROVIDER_DOMAIN, COCOA_DOMAIN, POSIX_DOMAIN, BRIDGE_DOMAIN, APP_DOMAIN`, `bridge_code::{MANAGER_UNAVAILABLE=1, STABILIZATION_TIMEOUT=2, RESOLVE_URL_TIMEOUT=3, SIGNAL_TIMEOUT=4, NO_IDENTIFIER=5}`, `app_code::{ENGINE_START=1, FINISH_READY=2, UNEXPECTED_BRIDGE_RETURN=3, LAUNCH_LOCATION=4}`.
  - `FpError::new(domain: &str, code: i64, message: impl Into<String>) -> FpError`, `.with_underlying(domain: &str, code: i64) -> FpError`, `FpError::app(code: i64, message: impl Into<String>) -> FpError`, `impl Display` (`"{message} ({domain} {code})"`).
  - `FailureRecord { reason: FinderFailureReason, domain: String, code: i64, at: i64 }` (Serialize/Deserialize, Clone, PartialEq).
  - `crate::finder_setup::policy::{classify(&FpError) -> FinderFailureReason, RetryPolicy, fp_code::*, COCOA_FILE_WRITE_FILE_EXISTS, POSIX_EEXIST, SIGNING_CODES}`.
  - `RetryPolicy { transient: Vec<Duration>, once: Vec<Duration>, stabilize_after_add: Duration, confirm_existing: Duration, user_disabled_poll: Duration }`, `Default`, `offsets(&self, FinderFailureReason) -> &[Duration]`, `max_attempts(&self, FinderFailureReason) -> u8`, `next_attempt_at(&self, reason, attempts_made: u8, started: Instant) -> Option<Instant>`.

- [ ] **Step 0: Record the baselines (first task only)**

From `$WT/src-tauri` at the untouched branch head (= `origin/main`):

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/baseline-clippy.log 2>&1; echo "rc=$?"
grep -c '^warning' $EVID/baseline-clippy.log > $EVID/baseline-clippy-warnings.txt; cat $EVID/baseline-clippy-warnings.txt
$LOCK cargo-build -- cargo test --locked > $EVID/baseline-cargo-test.log 2>&1; echo "rc=$?"
grep "test result:" $EVID/baseline-cargo-test.log
cd $WT && bun install --frozen-lockfile && bun test > $EVID/baseline-bun-test.log 2>&1; grep -E "^ *[0-9]+ (pass|fail)" $EVID/baseline-bun-test.log
```

Paste the warning count, every `test result:` line and the bun pass/fail lines into the task Notes under "Baseline (origin/main <sha>)".

- [ ] **Step 1: Add the vocabulary to `surfaces/phase.rs`**

Directly after the `FinderSetup` enum (it ends with `UserDisabled,\n}` at line ~40), insert:

```rust
impl FinderSetup {
    /// The snake_case name serde writes, for the lifecycle log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Adding => "adding",
            Self::Failed => "failed",
            Self::UserDisabled => "user_disabled",
        }
    }
}

/// Why Finder setup failed (spec 2026-10-06 §6.2). One closed vocabulary shared by the
/// reconciler, the popover snapshot, the lifecycle log, "Copy details" and the frontend
/// copy module (`src/finderSetupCopy.ts`). `UserDisabled` becomes the
/// `FinderSetup::UserDisabled` state, never `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinderFailureReason {
    ExtensionLoading,
    UserDisabled,
    NotInApplications,
    FolderTaken,
    Signing,
    Timeout,
    Unknown,
}

impl FinderFailureReason {
    pub const ALL: [Self; 7] = [
        Self::ExtensionLoading,
        Self::UserDisabled,
        Self::NotInApplications,
        Self::FolderTaken,
        Self::Signing,
        Self::Timeout,
        Self::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExtensionLoading => "extension_loading",
            Self::UserDisabled => "user_disabled",
            Self::NotInApplications => "not_in_applications",
            Self::FolderTaken => "folder_taken",
            Self::Signing => "signing",
            Self::Timeout => "timeout",
            Self::Unknown => "unknown",
        }
    }
}
```

At the end of `mod tests` in the same file, add:

```rust
    #[test]
    fn finder_names_match_what_serde_writes() {
        for setup in [FinderSetup::Ready, FinderSetup::Missing, FinderSetup::Adding, FinderSetup::Failed, FinderSetup::UserDisabled] {
            assert_eq!(serde_json::to_value(setup).unwrap(), serde_json::Value::String(setup.as_str().into()));
        }
        for reason in FinderFailureReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), serde_json::Value::String(reason.as_str().into()));
            let back: FinderFailureReason = serde_json::from_value(serde_json::Value::String(reason.as_str().into())).unwrap();
            assert_eq!(back, reason);
        }
    }
```

- [ ] **Step 2: Create `src-tauri/src/finder_setup/mod.rs`**

```rust
//! macOS Finder setup reconciler (spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md`,
//! plan `docs/superpowers/plans/2026-10-06-macos-finder-setup-reconciler.md`).
//!
//! Pure units (`error`, `policy`, `launch_location`, `core`) compile and are tested on every
//! platform. `driver` is the one async task; `macos_ports` (macOS only) is the only part that
//! touches FileProvider.framework, the engine and the app.

pub mod error;
pub mod policy;
```

- [ ] **Step 3: Create `src-tauri/src/finder_setup/error.rs`**

```rust
//! One NSError as the ObjC bridge saw it (spec §6.1), plus the errors we synthesize ourselves.

use crate::surfaces::phase::FinderFailureReason;

pub const FILE_PROVIDER_DOMAIN: &str = "NSFileProviderErrorDomain";
pub const COCOA_DOMAIN: &str = "NSCocoaErrorDomain";
pub const POSIX_DOMAIN: &str = "NSPOSIXErrorDomain";
/// Errors `FileProviderBridge.m` synthesizes when no NSError exists. Fixed codes below,
/// mirrored by the `BeebeebBridge*` enum in the bridge.
pub const BRIDGE_DOMAIN: &str = "io.beebeeb.bridge";
/// Errors the Rust side synthesizes (engine start, saving the sync folder, a bridge
/// return code the bridge does not document, a refused launch location).
pub const APP_DOMAIN: &str = "io.beebeeb.app";

pub mod bridge_code {
    pub const MANAGER_UNAVAILABLE: i64 = 1;
    pub const STABILIZATION_TIMEOUT: i64 = 2;
    pub const RESOLVE_URL_TIMEOUT: i64 = 3;
    pub const SIGNAL_TIMEOUT: i64 = 4;
    pub const NO_IDENTIFIER: i64 = 5;
}

pub mod app_code {
    pub const ENGINE_START: i64 = 1;
    pub const FINISH_READY: i64 = 2;
    pub const UNEXPECTED_BRIDGE_RETURN: i64 = 3;
    pub const LAUNCH_LOCATION: i64 = 4;
}

/// The domain and code of an `NSUnderlyingErrorKey` error.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FpErrorCode {
    pub domain: String,
    pub code: i64,
}

/// Classification reads only `domain`, `code` and `underlying` (spec §6.1). `message` goes
/// to the lifecycle log, redacted, and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FpError {
    pub domain: String,
    pub code: i64,
    pub message: String,
    pub underlying: Option<FpErrorCode>,
}

impl FpError {
    pub fn new(domain: &str, code: i64, message: impl Into<String>) -> Self {
        Self { domain: domain.to_string(), code, message: message.into(), underlying: None }
    }

    pub fn with_underlying(mut self, domain: &str, code: i64) -> Self {
        self.underlying = Some(FpErrorCode { domain: domain.to_string(), code });
        self
    }

    pub fn app(code: i64, message: impl Into<String>) -> Self {
        Self::new(APP_DOMAIN, code, message)
    }
}

/// Same shape the bridge's old flattened string had, so `%error` in existing `tracing` call
/// sites reads the same.
impl std::fmt::Display for FpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} {})", self.message, self.domain, self.code)
    }
}

/// The persisted last failure (spec §9: `finder_last_failure = { reason, domain, code, at }`).
/// `at` is Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FailureRecord {
    pub reason: FinderFailureReason,
    pub domain: String,
    pub code: i64,
    pub at: i64,
}
```

- [ ] **Step 4: Create `src-tauri/src/finder_setup/policy.rs` with its tests first, and the function bodies as `todo!()`**

```rust
//! Spec §6.2 (classification) and §7 (retry policy). The schedule and the clock live here
//! and nowhere else; the clock itself is injected by the caller (`started`, `now`).

use std::time::{Duration, Instant};

use super::error::{BRIDGE_DOMAIN, COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, FpError, POSIX_DOMAIN, bridge_code};
use crate::surfaces::phase::FinderFailureReason;

/// `NSFileProviderErrorDomain` codes, macOS 27 SDK `NSFileProviderError.h` (verified by the
/// lead on the build Mac, 2026-10-06).
pub mod fp_code {
    pub const PROVIDER_NOT_FOUND: i64 = -2001;
    pub const PROVIDER_TRANSLOCATED: i64 = -2002;
    pub const OLDER_EXTENSION_VERSION_RUNNING: i64 = -2003;
    pub const NEWER_EXTENSION_VERSION_FOUND: i64 = -2004;
    pub const CANNOT_SYNCHRONIZE: i64 = -2005;
    pub const DOMAIN_DISABLED: i64 = -2011;
    pub const PROVIDER_DOMAIN_TEMPORARILY_UNAVAILABLE: i64 = -2012;
    pub const PROVIDER_DOMAIN_NOT_FOUND: i64 = -2013;
    pub const APPLICATION_EXTENSION_NOT_FOUND: i64 = -2014;
}

/// `NSFileWriteFileExistsError`. PROVISIONAL until device check D0 confirms it (plan "Spec
/// issues" 5): the 0.8.11 app saved exactly this for the orphan collision on Guus's Mac
/// ("…a file with the same name already exists. (NSCocoaErrorDomain 516)", 2026-10-06
/// 13:05:09Z). D0 also records the underlying error, which may be the POSIX EEXIST below.
pub const COCOA_FILE_WRITE_FILE_EXISTS: i64 = 516;
pub const POSIX_EEXIST: i64 = 17;

/// `signing` (§6.2): entitlement, app-group or provisioning errors. EMPTY on purpose: the
/// bridge has no provisioning-specific path (plan "Spec issues" 6), so no code can be listed
/// from code. A captured code is added here together with its row in
/// `classify_covers_every_row_of_the_table`.
pub const SIGNING_CODES: &[(&str, i64)] = &[];

pub fn classify(error: &FpError) -> FinderFailureReason {
    todo!("Task 1 step 6")
}

/// Every attempt start, as an offset from the check's start (§7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// `extension_loading`, `timeout`: 0, 5, 15, 45 s.
    pub transient: Vec<Duration>,
    /// `unknown`: 0, 5 s (one silent retry).
    pub once: Vec<Duration>,
    /// How long one attempt waits for a freshly added domain to stabilize (≤ 10 s).
    pub stabilize_after_add: Duration,
    /// How long an existing registration gets to confirm it is stable (§5.5 step 3).
    pub confirm_existing: Duration,
    /// The read-only `userEnabled` poll while Beebeeb is turned off in System Settings.
    pub user_disabled_poll: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        todo!("Task 1 step 6")
    }
}

impl RetryPolicy {
    pub fn offsets(&self, reason: FinderFailureReason) -> &[Duration] {
        todo!("Task 1 step 6")
    }

    pub fn max_attempts(&self, reason: FinderFailureReason) -> u8 {
        todo!("Task 1 step 6")
    }

    /// When attempt number `attempts_made + 1` may start, or `None` once the budget for
    /// `reason` is spent. An attempt that is still running at that moment simply starts the
    /// next one late; the caller takes `max(at, now)`.
    pub fn next_attempt_at(&self, reason: FinderFailureReason, attempts_made: u8, started: Instant) -> Option<Instant> {
        todo!("Task 1 step 6")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{APP_DOMAIN, app_code};
    use FinderFailureReason::*;

    fn e(domain: &str, code: i64) -> FpError {
        FpError::new(domain, code, "fixture message")
    }

    /// Spec §6.2, row by row: each entry is one row's macOS signal.
    #[test]
    fn classify_covers_every_row_of_the_table() {
        let rows: Vec<(&str, FpError, FinderFailureReason)> = vec![
            ("-2001 ProviderNotFound", e(FILE_PROVIDER_DOMAIN, -2001), ExtensionLoading),
            ("-2003 OlderExtensionVersionRunning", e(FILE_PROVIDER_DOMAIN, -2003), ExtensionLoading),
            ("-2004 NewerExtensionVersionFound", e(FILE_PROVIDER_DOMAIN, -2004), ExtensionLoading),
            ("-2012 ProviderDomainTemporarilyUnavailable", e(FILE_PROVIDER_DOMAIN, -2012), ExtensionLoading),
            ("-2014 ApplicationExtensionNotFound", e(FILE_PROVIDER_DOMAIN, -2014), ExtensionLoading),
            ("-2011 DomainDisabled", e(FILE_PROVIDER_DOMAIN, -2011), UserDisabled),
            ("-2002 ProviderTranslocated", e(FILE_PROVIDER_DOMAIN, -2002), NotInApplications),
            ("folder taken, top level (0.8.11 evidence)", e(COCOA_DOMAIN, 516), FolderTaken),
            ("folder taken, underlying EEXIST", e("SomeWrapperDomain", 1).with_underlying(POSIX_DOMAIN, 17), FolderTaken),
            ("stabilization timeout", e(BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT), Timeout),
        ];
        for (name, error, expected) in &rows {
            assert_eq!(classify(error), *expected, "{name}");
        }
    }

    #[test]
    fn everything_else_is_unknown() {
        for error in [
            e(FILE_PROVIDER_DOMAIN, -2005), // CannotSynchronize: verified code, no §6.2 row (Spec issue 10)
            e(FILE_PROVIDER_DOMAIN, -2013), // ProviderDomainNotFound: same
            e(FILE_PROVIDER_DOMAIN, -1000),
            e(COCOA_DOMAIN, 4099),
            e(POSIX_DOMAIN, 13),
            e(BRIDGE_DOMAIN, bridge_code::MANAGER_UNAVAILABLE),
            e(APP_DOMAIN, app_code::ENGINE_START),
            e("", 0),
        ] {
            assert_eq!(classify(&error), Unknown, "{error:?}");
        }
    }

    #[test]
    fn the_message_is_never_read() {
        // The old macOS classifier matched English fragments of our own copy. The same domain +
        // code with any message must classify the same.
        for message in [
            "Provisioning profile entitlement app-group mismatch",
            "Timed out waiting",
            "Beebeeb is turned off in System Settings",
            "",
        ] {
            assert_eq!(classify(&FpError::new("SomeDomain", 7, message)), Unknown, "{message}");
            assert_eq!(classify(&FpError::new(FILE_PROVIDER_DOMAIN, -2001, message)), ExtensionLoading, "{message}");
        }
    }

    #[test]
    fn signing_has_no_code_until_one_is_captured() {
        assert!(SIGNING_CODES.is_empty(), "add the captured code's row to classify_covers_every_row_of_the_table");
        for code in -2020..=-2000 {
            assert_ne!(classify(&e(FILE_PROVIDER_DOMAIN, code)), Signing, "{code}");
        }
    }

    #[test]
    fn retry_classes_follow_section_7() {
        let policy = RetryPolicy::default();
        let secs = |reason| policy.offsets(reason).iter().map(|d| d.as_secs()).collect::<Vec<_>>();
        assert_eq!(secs(ExtensionLoading), vec![0, 5, 15, 45]);
        assert_eq!(secs(Timeout), vec![0, 5, 15, 45]);
        assert_eq!(secs(Unknown), vec![0, 5]);
        for reason in [FolderTaken, Signing, NotInApplications, UserDisabled] {
            assert_eq!(secs(reason), vec![0], "{reason:?}");
        }
        assert_eq!(policy.max_attempts(ExtensionLoading), 4);
        assert_eq!(policy.max_attempts(Unknown), 2);
        assert_eq!(policy.max_attempts(FolderTaken), 1);
        assert_eq!(policy.stabilize_after_add, Duration::from_secs(10));
        assert_eq!(policy.confirm_existing, Duration::from_secs(2));
        assert_eq!(policy.user_disabled_poll, Duration::from_secs(3));
    }

    #[test]
    fn next_attempt_at_counts_from_the_check_start_and_ends_with_the_budget() {
        let policy = RetryPolicy::default();
        let t0 = Instant::now();
        let at = |reason: FinderFailureReason, made: u8| policy.next_attempt_at(reason, made, t0).map(|i| (i - t0).as_secs());
        assert_eq!((1u8..=4).map(|made| at(Timeout, made)).collect::<Vec<_>>(), vec![Some(5), Some(15), Some(45), None]);
        assert_eq!(at(Unknown, 1), Some(5));
        assert_eq!(at(Unknown, 2), None);
        assert_eq!(at(FolderTaken, 1), None);
    }
}
```

Register the module in `src-tauri/src/lib.rs`, after `mod engine_status;`:

```rust
// Spec 2026-10-06 (macOS Finder setup reconciler). `allow(dead_code)` only until Task 8 wires
// the driver; Task 8 narrows it to non-macOS targets.
#[allow(dead_code)]
mod finder_setup;
```

- [ ] **Step 5: Run the tests and watch them fail**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup::policy > $EVID/t1-red.log 2>&1; echo "rc=$?"; grep -E "test result:|panicked|not yet implemented" $EVID/t1-red.log | head
```

Expected: `rc=101`, and every test panics with `not yet implemented: Task 1 step 6`. Paste the `test result: FAILED. 0 passed; 6 failed` line into Notes.

- [ ] **Step 6: Implement**

Replace the four `todo!()` bodies:

```rust
pub fn classify(error: &FpError) -> FinderFailureReason {
    use FinderFailureReason::*;
    if error.domain == FILE_PROVIDER_DOMAIN {
        match error.code {
            fp_code::PROVIDER_NOT_FOUND
            | fp_code::OLDER_EXTENSION_VERSION_RUNNING
            | fp_code::NEWER_EXTENSION_VERSION_FOUND
            | fp_code::PROVIDER_DOMAIN_TEMPORARILY_UNAVAILABLE
            | fp_code::APPLICATION_EXTENSION_NOT_FOUND => return ExtensionLoading,
            fp_code::DOMAIN_DISABLED => return UserDisabled,
            fp_code::PROVIDER_TRANSLOCATED => return NotInApplications,
            _ => {}
        }
    }
    let is = |domain: &str, code: i64| {
        (error.domain == domain && error.code == code)
            || error.underlying.as_ref().is_some_and(|u| u.domain == domain && u.code == code)
    };
    if is(COCOA_DOMAIN, COCOA_FILE_WRITE_FILE_EXISTS) || is(POSIX_DOMAIN, POSIX_EEXIST) {
        return FolderTaken;
    }
    if SIGNING_CODES.iter().any(|(domain, code)| is(domain, *code)) {
        return Signing;
    }
    if error.domain == BRIDGE_DOMAIN && error.code == bridge_code::STABILIZATION_TIMEOUT {
        return Timeout;
    }
    Unknown
}

/// One attempt, for reasons that are never retried.
const ONE_ATTEMPT: [Duration; 1] = [Duration::ZERO];

impl Default for RetryPolicy {
    fn default() -> Self {
        let s = Duration::from_secs;
        Self {
            transient: vec![s(0), s(5), s(15), s(45)],
            once: vec![s(0), s(5)],
            stabilize_after_add: s(10),
            confirm_existing: s(2),
            user_disabled_poll: s(3),
        }
    }
}

impl RetryPolicy {
    pub fn offsets(&self, reason: FinderFailureReason) -> &[Duration] {
        use FinderFailureReason::*;
        match reason {
            ExtensionLoading | Timeout => &self.transient,
            Unknown => &self.once,
            UserDisabled | NotInApplications | FolderTaken | Signing => &ONE_ATTEMPT,
        }
    }

    pub fn max_attempts(&self, reason: FinderFailureReason) -> u8 {
        u8::try_from(self.offsets(reason).len()).unwrap_or(u8::MAX)
    }

    pub fn next_attempt_at(&self, reason: FinderFailureReason, attempts_made: u8, started: Instant) -> Option<Instant> {
        self.offsets(reason).get(usize::from(attempts_made)).map(|offset| started + *offset)
    }
}
```

- [ ] **Step 7: Run green**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup::policy > $EVID/t1-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t1-green.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib surfaces::phase > $EVID/t1-phase.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t1-phase.log
```

Expected: `test result: ok. 6 passed; 0 failed`, and the phase run passes with one test more than before.

- [ ] **Step 8: Mutation check (spec §13.1, required)**

Change the `fp_code::DOMAIN_DISABLED => return UserDisabled,` arm to `=> return ExtensionLoading,` and rerun the green command into `$EVID/t1-mutation.log`. Expected: `classify_covers_every_row_of_the_table` fails with `-2011 DomainDisabled`, and **only** that test fails. Paste the failing assertion into Notes, revert with your editor (not `git checkout`), and rerun green.

- [ ] **Step 9: Compile all targets and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t1-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/finder_setup/mod.rs src-tauri/src/finder_setup/error.rs src-tauri/src/finder_setup/policy.rs
git commit -m "finder setup: reason vocabulary, FpError, classifier and retry policy (spec A §6, §7)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/finder_setup/mod.rs src-tauri/src/finder_setup/error.rs src-tauri/src/finder_setup/policy.rs src-tauri/src/surfaces/phase.rs src-tauri/src/lib.rs
git show --stat HEAD
```

Expected: `rc=0`, and `git show --stat HEAD` lists exactly those 5 files.

---

## Task 2: `launch_location` (pure)

**Lane R.**

**Files:**
- Create: `src-tauri/src/finder_setup/launch_location.rs`
- Modify: `src-tauri/src/finder_setup/mod.rs` (add `pub mod launch_location;`)
- Modify: `src-tauri/src/ipc_socket.rs:646` (`fn macos_real_home_dir` becomes `pub(crate) fn macos_real_home_dir`)

**Interfaces:**
- Consumes: nothing.
- Produces: `crate::finder_setup::launch_location::{LaunchLocation, launch_location, bundle_path_from_exe}`, plus `current() -> LaunchLocation` and `current_bundle_path() -> Option<PathBuf>` (both `#[cfg(target_os = "macos")]`).
  - `LaunchLocation` is `Copy`, Serialize snake_case: `Applications | Translocated | DiskImage | Elsewhere`; `fn allows_add(self) -> bool`; `fn as_str(self) -> &'static str`.
  - `launch_location(bundle: &Path, real_home: &Path) -> LaunchLocation`. This adds `real_home` to the spec's `launch_location(&Path)` signature, so the function stays pure under the sandbox. Justification: `$HOME` is the app container (Spec issue 4), so `~/Applications` has to come from `ipc_socket::macos_real_home_dir()`.

- [ ] **Step 1: Write the module with tests and `todo!()` bodies**

```rust
//! Where the app runs from (spec §6.2 "The launch-location check"). Pure: the bundle path and
//! the REAL home are passed in. Under the app sandbox `$HOME` is the app container, so the
//! caller passes `ipc_socket::macos_real_home_dir()`, never `dirs::home_dir()`.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchLocation {
    /// `/Applications/…` or `~/Applications/…`.
    Applications,
    /// Gatekeeper App Translocation (a quarantined app opened where it was downloaded).
    Translocated,
    /// Anything under `/Volumes/` (the mounted disk image).
    DiskImage,
    /// Anywhere else: proceed and let macOS decide (a -2002 is still classified).
    Elsewhere,
}

impl LaunchLocation {
    /// Whether `addDomain` may be called from here.
    pub fn allows_add(self) -> bool {
        todo!("Task 2 step 3")
    }

    pub fn as_str(self) -> &'static str {
        todo!("Task 2 step 3")
    }
}

pub fn launch_location(bundle: &Path, real_home: &Path) -> LaunchLocation {
    todo!("Task 2 step 3")
}

/// `…/Beebeeb.app/Contents/MacOS/beebeeb` → `…/Beebeeb.app`; `None` for a bare dev binary
/// (`target/debug/beebeeb`), which therefore counts as `Elsewhere`.
pub fn bundle_path_from_exe(exe: &Path) -> Option<PathBuf> {
    todo!("Task 2 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/Users/sam";

    fn at(path: &str) -> LaunchLocation {
        launch_location(Path::new(path), Path::new(HOME))
    }

    #[test]
    fn the_five_paths_of_section_13() {
        assert_eq!(at("/Applications/Beebeeb.app"), LaunchLocation::Applications);
        assert_eq!(at("/Users/sam/Applications/Beebeeb.app"), LaunchLocation::Applications);
        assert_eq!(
            at("/private/var/folders/x7/abc123/T/AppTranslocation/6F0E1C2D-0000-4000-8000-000000000000/d/Beebeeb.app"),
            LaunchLocation::Translocated
        );
        assert_eq!(at("/Volumes/Beebeeb/Beebeeb.app"), LaunchLocation::DiskImage);
        assert_eq!(at("/Users/sam/Downloads/Beebeeb.app"), LaunchLocation::Elsewhere);
    }

    #[test]
    fn only_translocated_and_disk_image_block_add() {
        assert!(LaunchLocation::Applications.allows_add());
        assert!(LaunchLocation::Elsewhere.allows_add());
        assert!(!LaunchLocation::Translocated.allows_add());
        assert!(!LaunchLocation::DiskImage.allows_add());
    }

    #[test]
    fn matching_is_by_path_component_not_by_text() {
        assert_eq!(at("/ApplicationsOld/Beebeeb.app"), LaunchLocation::Elsewhere);
        assert_eq!(at("/Users/sam/Applications Backup/Beebeeb.app"), LaunchLocation::Elsewhere);
        assert_eq!(at("/Users/samuel/Applications/Beebeeb.app"), LaunchLocation::Elsewhere);
        assert_eq!(at("/Applications/Utilities/Beebeeb.app"), LaunchLocation::Applications);
    }

    #[test]
    fn home_applications_needs_the_real_home_not_the_sandbox_container() {
        assert_eq!(
            at("/Users/sam/Library/Containers/io.beebeeb.app/Data/Applications/Beebeeb.app"),
            LaunchLocation::Elsewhere
        );
    }

    /// Spec §6.2 taken literally (plan "Spec issues" 11): an app in an external disk's
    /// Applications folder is refused too. Change this test only with Guus's ruling.
    #[test]
    fn volumes_is_refused_even_under_an_applications_folder() {
        assert_eq!(at("/Volumes/Data/Applications/Beebeeb.app"), LaunchLocation::DiskImage);
    }

    #[test]
    fn bundle_path_from_exe_finds_the_app_bundle() {
        assert_eq!(
            bundle_path_from_exe(Path::new("/Applications/Beebeeb.app/Contents/MacOS/beebeeb")),
            Some(PathBuf::from("/Applications/Beebeeb.app"))
        );
        assert_eq!(bundle_path_from_exe(Path::new("/Users/sam/src/desktop/src-tauri/target/debug/beebeeb")), None);
    }

    #[test]
    fn names_match_what_serde_writes() {
        for location in [LaunchLocation::Applications, LaunchLocation::Translocated, LaunchLocation::DiskImage, LaunchLocation::Elsewhere] {
            assert_eq!(serde_json::to_value(location).unwrap(), serde_json::Value::String(location.as_str().into()));
        }
    }
}
```

Add `pub mod launch_location;` to `finder_setup/mod.rs`.

- [ ] **Step 2: Run RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup::launch_location > $EVID/t2-red.log 2>&1; echo "rc=$?"; grep -E "test result:" $EVID/t2-red.log
```

Expected: `rc=101`, `test result: FAILED. 0 passed; 7 failed` (all `not yet implemented`).

- [ ] **Step 3: Implement**

```rust
impl LaunchLocation {
    pub fn allows_add(self) -> bool {
        !matches!(self, Self::Translocated | Self::DiskImage)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applications => "applications",
            Self::Translocated => "translocated",
            Self::DiskImage => "disk_image",
            Self::Elsewhere => "elsewhere",
        }
    }
}

pub fn launch_location(bundle: &Path, real_home: &Path) -> LaunchLocation {
    if bundle.components().any(|c| c == Component::Normal("AppTranslocation".as_ref())) {
        return LaunchLocation::Translocated;
    }
    if bundle.starts_with("/Volumes") {
        return LaunchLocation::DiskImage;
    }
    if bundle.starts_with("/Applications") || bundle.starts_with(real_home.join("Applications")) {
        return LaunchLocation::Applications;
    }
    LaunchLocation::Elsewhere
}

pub fn bundle_path_from_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// The running app's bundle, read when needed ("Show in Finder").
#[cfg(target_os = "macos")]
pub fn current_bundle_path() -> Option<PathBuf> {
    std::env::current_exe().ok().as_deref().and_then(bundle_path_from_exe)
}

/// The running app's location, read once at launch.
#[cfg(target_os = "macos")]
pub fn current() -> LaunchLocation {
    match current_bundle_path() {
        Some(bundle) => launch_location(&bundle, &crate::ipc_socket::macos_real_home_dir()),
        None => LaunchLocation::Elsewhere,
    }
}
```

In `src-tauri/src/ipc_socket.rs:646` change `fn macos_real_home_dir()` to `pub(crate) fn macos_real_home_dir()`.

- [ ] **Step 4: Run GREEN**

Same command as step 2 into `$EVID/t2-green.log`. Expected `test result: ok. 7 passed; 0 failed`.

- [ ] **Step 5: Mutation check**

Delete the `if bundle.starts_with("/Volumes") { … }` block and rerun into `$EVID/t2-mutation.log`. Expected failures: `the_five_paths_of_section_13` and `volumes_is_refused_even_under_an_applications_folder`. Paste them into Notes, restore the block, rerun green.

- [ ] **Step 6: Check and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t2-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/finder_setup/launch_location.rs
git commit -m "finder setup: launch_location, sandbox-safe ~/Applications (spec A §6.2)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/finder_setup/launch_location.rs src-tauri/src/finder_setup/mod.rs src-tauri/src/ipc_socket.rs
git show --stat HEAD
```

---

## Task 3: The reconciler core (pure state machine)

**Lane R.** This is the heart of spec §5. No OS, no clock, no I/O.

**Files:**
- Create: `src-tauri/src/finder_setup/core.rs`
- Modify: `src-tauri/src/finder_setup/mod.rs` (add `pub mod core;`)

**Interfaces:**
- Consumes (Tasks 1, 2): `FpError`, `app_code::{LAUNCH_LOCATION, UNEXPECTED_BRIDGE_RETURN}`, `policy::{classify, RetryPolicy}`, `LaunchLocation`, `FinderSetup`, `FinderFailureReason`.
- Produces (used by Tasks 5, 7, 8, 9):
  - `Trigger { Launch, KeysArrived, UserEnabledFlipped, TryAgain, SignOut, Repair, Lock }`, `Trigger::as_str`.
  - `Wanted { Present, Absent, NoAction }`; `SessionFacts { vault_unlocked: bool, auth_present: bool, signed_out_by_choice: bool }`; `fn wanted(SessionFacts) -> Wanted`.
  - `DomainState { Enabled, Disabled, NotRegistered }`; `Observation { facts: SessionFacts, domain: Result<DomainState, FpError> }`.
  - `Op { Observe, StartEngine, AddDomain, ReadDomain, WaitStable(Duration), FinishReady, StopEngine, RemoveDomain }`.
  - `OpResult { Observed(Observation), EngineStarted(Result<bool, FpError>) /* true = this op started a new engine */, Added(Result<(), FpError>), Domain(Result<DomainState, FpError>), Stable(Result<(), FpError>), Finished(Result<(), FpError>), EngineStopped, Removed(Result<(), FpError>) }`.
  - `Input { Trigger(Trigger), Tick { window_visible: bool }, AppActivated, Done(OpResult) }`.
  - `Transition { from, to, reason, error, attempt, max_attempts }`.
  - `Effect { Publish, TriggerReceived(Trigger), Transition(Transition), PersistFailure(Option<(FinderFailureReason, FpError)>), PersistSignedOutByChoice(bool), RemoveFinished(Result<(), FpError>) }`.
  - `Next { Run(Op), WakeAt(Instant), Idle }`.
  - `CoreState` (pub fields `setup, reason, last_error, attempt, max_attempts, launch`), `CoreState::new(LaunchLocation)`, `CoreState::is_settled(&self) -> bool`.
  - `fn step(state: CoreState, input: Input, now: Instant, policy: &RetryPolicy) -> (CoreState, Vec<Effect>)`.
  - `fn next_op(state: &mut CoreState, now: Instant, policy: &RetryPolicy) -> Next`.
  - The spec's `step(state, event, observation, now)` is refined into these two pure functions. Observations arrive as `Input::Done(OpResult::Observed(..))`, and `next_op` answers "what now" only between operations. That is what lets the driver guarantee one operation at a time and apply events (sign-out, lock) before the next operation starts.

**Invariant the driver relies on:** after `next_op` returns `Run(op)`, the next input fed to `step` is `Done(result of op)`. Events are only fed between operations.

- [ ] **Step 1: Write `core.rs` with the full test module first, and `todo!()` in `step` and `next_op`**

Write the file below exactly, except that the bodies of `step` and `next_op` are `todo!("Task 3 step 3")` for now. All private helpers (`on_trigger` … `set_state`) go in at step 3.

```rust
//! Spec 2026-10-06 §5 (`finder_setup::core`): what the Finder reconciler decides. Pure: no OS,
//! no clock, no I/O. The driver feeds one `Input` at a time and runs the one `Op` that
//! `next_op` asks for. While that op runs nothing else is fed (events wait in the driver's
//! channel), so an add and a remove can never overlap (§5.4).

use std::time::{Duration, Instant};

use super::error::{FpError, app_code};
use super::launch_location::LaunchLocation;
use super::policy::{self, RetryPolicy};
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

/// What starts a check (§5.3), the two removals, and lock. Also the lifecycle log's
/// "trigger received" vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Launch,
    KeysArrived,
    UserEnabledFlipped,
    TryAgain,
    SignOut,
    Repair,
    Lock,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::KeysArrived => "keys_arrived",
            Self::UserEnabledFlipped => "user_enabled_flipped",
            Self::TryAgain => "try_again",
            Self::SignOut => "sign_out",
            Self::Repair => "repair",
            Self::Lock => "lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Present,
    Absent,
    NoAction,
}

/// The facts `Wanted` is derived from (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionFacts {
    /// This Mac has the keys in memory (`AccountRuntime::session` is `Some`).
    pub vault_unlocked: bool,
    /// A session token is stored (`AppState::auth_present`).
    pub auth_present: bool,
    /// The last sign-out was the person's choice and no sign-in followed
    /// (`DesktopConfig::finder_signed_out_by_choice`). A startup 401 that discards a revoked
    /// token does NOT set it, so ruling R2 keeps Beebeeb in Finder (plan "Spec issues" 2).
    pub signed_out_by_choice: bool,
}

/// §5.1. A revoked session (`auth_expired`) is not an input at all: with the keys in memory it
/// is `Present` (R2).
pub fn wanted(facts: SessionFacts) -> Wanted {
    if facts.vault_unlocked {
        Wanted::Present
    } else if !facts.auth_present && facts.signed_out_by_choice {
        Wanted::Absent
    } else {
        Wanted::NoAction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainState {
    Enabled,
    Disabled,
    NotRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub facts: SessionFacts,
    pub domain: Result<DomainState, FpError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Observe,
    StartEngine,
    AddDomain,
    ReadDomain,
    WaitStable(Duration),
    FinishReady,
    StopEngine,
    RemoveDomain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpResult {
    Observed(Observation),
    /// `Ok(true)`: this op started a new engine; `Ok(false)`: one was already running.
    EngineStarted(Result<bool, FpError>),
    Added(Result<(), FpError>),
    Domain(Result<DomainState, FpError>),
    Stable(Result<(), FpError>),
    Finished(Result<(), FpError>),
    EngineStopped,
    Removed(Result<(), FpError>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Trigger(Trigger),
    /// A timer the core asked for (`Next::WakeAt`) fired.
    Tick { window_visible: bool },
    AppActivated,
    Done(OpResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub from: FinderSetup,
    pub to: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub error: Option<FpError>,
    pub attempt: u8,
    pub max_attempts: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// The published view changed: emit `finder-setup-changed`.
    Publish,
    TriggerReceived(Trigger),
    Transition(Transition),
    /// `Some` when a check ends in `Failed`, `None` when it ends in `Ready`.
    PersistFailure(Option<(FinderFailureReason, FpError)>),
    PersistSignedOutByChoice(bool),
    /// A removal requested by sign-out or Repair ran: resolve whoever waits for it.
    RemoveFinished(Result<(), FpError>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Run(Op),
    WakeAt(Instant),
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    AfterAdd,
    ConfirmExisting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    Failed,
    UserDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Observe,
    StartEngine { add: bool },
    Add,
    ReadAfterAdd,
    Wait(Wait),
    Finish,
    StopEngine(Landing),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Running(Step),
    Backoff(Instant),
    /// `requested`: sign-out or Repair waits for it. `then_check`: Repair's "then one fresh check".
    Removing { requested: bool, then_check: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreState {
    pub setup: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub last_error: Option<FpError>,
    pub attempt: u8,
    pub max_attempts: u8,
    pub launch: LaunchLocation,
    phase: Phase,
    awaiting: bool,
    check_started: Option<Instant>,
    check_started_engine: bool,
    /// A trigger arrived during a check: run exactly one more afterwards (§5.4).
    check_again: bool,
    /// Set by sign-out and lock; only `Launch` or `KeysArrived` clears it. While held no check
    /// starts, so nothing re-adds the domain or starts the engine in the gap between the
    /// reconciler finishing and the caller clearing the session.
    held: bool,
    poll_due: Option<Instant>,
    poll_pending: bool,
}

impl CoreState {
    pub fn new(launch: LaunchLocation) -> Self {
        Self {
            setup: FinderSetup::Missing,
            reason: missing_reason(launch),
            last_error: None,
            attempt: 0,
            max_attempts: 0,
            launch,
            phase: Phase::Idle,
            awaiting: false,
            check_started: None,
            check_started_engine: false,
            check_again: false,
            held: false,
            poll_due: None,
            poll_pending: false,
        }
    }

    /// Nothing runs and nothing is scheduled except, possibly, the UserDisabled poll.
    pub fn is_settled(&self) -> bool {
        self.phase == Phase::Idle && !self.awaiting && !self.poll_pending
    }
}

/// From a disk image or a translocated copy the reason is known before any check (§6.2:
/// "exposed in `finder_setup_state` before sign-in").
fn missing_reason(launch: LaunchLocation) -> Option<FinderFailureReason> {
    if launch.allows_add() { None } else { Some(FinderFailureReason::NotInApplications) }
}

pub fn step(mut state: CoreState, input: Input, now: Instant, policy: &RetryPolicy) -> (CoreState, Vec<Effect>) {
    todo!("Task 3 step 3")
}

pub fn next_op(state: &mut CoreState, now: Instant, policy: &RetryPolicy) -> Next {
    todo!("Task 3 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{BRIDGE_DOMAIN, COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, bridge_code};
    use DomainState::{Disabled, Enabled, NotRegistered};
    use FinderSetup::{Adding, Failed, Missing, Ready, UserDisabled};
    use LaunchLocation::{Applications, DiskImage};

    const SIGNED_IN: SessionFacts = SessionFacts { vault_unlocked: true, auth_present: true, signed_out_by_choice: false };
    const KEYS_MISSING: SessionFacts = SessionFacts { vault_unlocked: false, auth_present: true, signed_out_by_choice: false };
    const SIGNED_OUT_BY_CHOICE: SessionFacts = SessionFacts { vault_unlocked: false, auth_present: false, signed_out_by_choice: true };
    const SIGNED_OUT_BY_401: SessionFacts = SessionFacts { vault_unlocked: false, auth_present: false, signed_out_by_choice: false };

    fn fp(code: i64) -> FpError {
        FpError::new(FILE_PROVIDER_DOMAIN, code, "fixture")
    }

    fn stabilization_timeout() -> FpError {
        FpError::new(BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT, "fixture")
    }

    /// What the fake OS answers. `wait_secs` is how long one `WaitStable` takes on the fake
    /// clock; every other op takes no time.
    #[derive(Clone)]
    struct World {
        facts: SessionFacts,
        domain: Result<DomainState, FpError>,
        add: Result<(), FpError>,
        after_add: Result<DomainState, FpError>,
        stable: Result<(), FpError>,
        wait_secs: u64,
    }

    impl World {
        fn ok(facts: SessionFacts, domain: DomainState) -> Self {
            Self { facts, domain: Ok(domain), add: Ok(()), after_add: Ok(Enabled), stable: Ok(()), wait_secs: 0 }
        }
    }

    fn answer(world: &World, op: Op) -> OpResult {
        match op {
            Op::Observe => OpResult::Observed(Observation { facts: world.facts, domain: world.domain.clone() }),
            Op::StartEngine => OpResult::EngineStarted(Ok(true)),
            Op::AddDomain => OpResult::Added(world.add.clone()),
            Op::ReadDomain => OpResult::Domain(world.after_add.clone()),
            Op::WaitStable(_) => OpResult::Stable(world.stable.clone()),
            Op::FinishReady => OpResult::Finished(Ok(())),
            Op::StopEngine => OpResult::EngineStopped,
            Op::RemoveDomain => OpResult::Removed(Ok(())),
        }
    }

    /// A miniature driver on an injected clock.
    struct Sim {
        s: CoreState,
        t0: Instant,
        now: Instant,
        policy: RetryPolicy,
        ops: Vec<(u64, Op)>,
        fx: Vec<Effect>,
    }

    impl Sim {
        fn new(launch: LaunchLocation) -> Self {
            let t0 = Instant::now();
            Self { s: CoreState::new(launch), t0, now: t0, policy: RetryPolicy::default(), ops: Vec::new(), fx: Vec::new() }
        }
        fn secs(&self) -> u64 {
            (self.now - self.t0).as_secs()
        }
        fn feed(&mut self, input: Input) {
            let (s, fx) = step(self.s.clone(), input, self.now, &self.policy);
            self.s = s;
            self.fx.extend(fx);
        }
        fn trigger(&mut self, trigger: Trigger) {
            self.feed(Input::Trigger(trigger));
        }
        fn next(&mut self) -> Next {
            next_op(&mut self.s, self.now, &self.policy)
        }
        fn step_op(&mut self, world: &World) -> Op {
            match self.next() {
                Next::Run(op) => {
                    self.ops.push((self.secs(), op));
                    let result = answer(world, op);
                    if matches!(op, Op::WaitStable(_)) {
                        self.now += Duration::from_secs(world.wait_secs);
                    }
                    self.feed(Input::Done(result));
                    op
                }
                other => panic!("expected an operation, got {other:?}"),
            }
        }
        /// Answer ops from `world` (and let backoff timers fire) until the core settles.
        fn run(&mut self, world: &World) {
            for _ in 0..500 {
                if self.s.is_settled() {
                    return;
                }
                match self.next() {
                    Next::Idle => return,
                    Next::WakeAt(at) => {
                        self.now = self.now.max(at);
                        self.feed(Input::Tick { window_visible: false });
                    }
                    Next::Run(op) => {
                        self.ops.push((self.secs(), op));
                        let result = answer(world, op);
                        if matches!(op, Op::WaitStable(_)) {
                            self.now += Duration::from_secs(world.wait_secs);
                        }
                        self.feed(Input::Done(result));
                    }
                }
            }
            panic!("the core did not settle: {:?}", self.s);
        }
        fn kinds(&self) -> Vec<Op> {
            self.ops.iter().map(|(_, op)| *op).collect()
        }
        fn count(&self, op: Op) -> usize {
            self.ops.iter().filter(|(_, o)| *o == op).count()
        }
        fn attempt_starts(&self) -> Vec<u64> {
            self.ops.iter().filter(|(_, op)| *op == Op::Observe).map(|(t, _)| *t).collect()
        }
    }

    const WAIT_10: Op = Op::WaitStable(Duration::from_secs(10));
    const WAIT_2: Op = Op::WaitStable(Duration::from_secs(2));

    #[test]
    fn wanted_follows_section_5_1() {
        assert_eq!(wanted(SIGNED_IN), Wanted::Present);
        assert_eq!(wanted(SessionFacts { vault_unlocked: true, auth_present: false, signed_out_by_choice: true }), Wanted::Present);
        assert_eq!(wanted(KEYS_MISSING), Wanted::NoAction);
        assert_eq!(wanted(SIGNED_OUT_BY_CHOICE), Wanted::Absent);
        assert_eq!(wanted(SIGNED_OUT_BY_401), Wanted::NoAction, "R2: a startup 401 is not a sign-out by choice");
    }

    #[test]
    fn present_and_not_registered_adds_then_lands_ready() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::ReadDomain, WAIT_10, Op::FinishReady]);
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.s.reason, None);
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    #[test]
    fn present_and_registered_confirms_without_adding_and_ready_never_flickers() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, WAIT_2, Op::FinishReady]);
        assert_eq!(sim.s.setup, Ready);
        // A second launch-like trigger on a Ready domain never publishes Adding.
        sim.fx.clear();
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert!(!sim.fx.iter().any(|e| matches!(e, Effect::Transition(t) if t.to == Adding)), "{:?}", sim.fx);
        assert_eq!(sim.count(Op::AddDomain), 0);
    }

    #[test]
    fn present_and_turned_off_lands_user_disabled_without_adding() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::UserDisabled));
    }

    #[test]
    fn turned_off_right_after_add_stops_the_engine_it_started() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { after_add: Ok(Disabled), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::ReadDomain, Op::StopEngine]);
        assert_eq!(sim.s.setup, UserDisabled);
    }

    #[test]
    fn domain_disabled_from_add_is_user_disabled_not_failed() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { add: Err(fp(-2011)), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.count(Op::AddDomain), 1, "-2011 is not retried");
    }

    #[test]
    fn a_failed_read_after_add_is_not_evidence_and_still_waits() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { after_add: Err(fp(-1000)), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::ReadDomain, WAIT_10, Op::FinishReady]);
        assert_eq!(sim.s.setup, Ready);
    }

    /// §13.1 "table tests for every trigger × every observation".
    #[test]
    fn every_trigger_against_every_observation_ends_in_a_state_a_person_can_act_on() {
        let observations: Vec<(&str, World, FinderSetup, usize)> = vec![
            // (name, world, final state, RemoveDomain count)
            ("signed in, not registered", World::ok(SIGNED_IN, NotRegistered), Ready, 0),
            ("signed in, registered", World::ok(SIGNED_IN, Enabled), Ready, 0),
            ("signed in, turned off", World::ok(SIGNED_IN, Disabled), UserDisabled, 0),
            ("signed in, turned off right after add", World { after_add: Ok(Disabled), ..World::ok(SIGNED_IN, NotRegistered) }, UserDisabled, 0),
            ("signed in, folder taken", World { add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")), ..World::ok(SIGNED_IN, NotRegistered) }, Failed, 0),
            ("signed in, lookup keeps failing", World { domain: Err(fp(-2001)), ..World::ok(SIGNED_IN, Enabled) }, Failed, 0),
            ("signed in, never stable", World { stable: Err(stabilization_timeout()), ..World::ok(SIGNED_IN, NotRegistered) }, Failed, 0),
            ("keys missing", World::ok(KEYS_MISSING, Enabled), Missing, 0),
            ("signed out by choice, still registered", World::ok(SIGNED_OUT_BY_CHOICE, Enabled), Missing, 1),
            ("signed out by choice, gone", World::ok(SIGNED_OUT_BY_CHOICE, NotRegistered), Missing, 0),
            ("revoked at startup (401), registered", World::ok(SIGNED_OUT_BY_401, Enabled), Missing, 0),
        ];
        for trigger in [Trigger::Launch, Trigger::KeysArrived, Trigger::TryAgain] {
            for (name, world, expected, removes) in &observations {
                let mut sim = Sim::new(Applications);
                sim.trigger(trigger);
                sim.run(world);
                assert_eq!(sim.s.setup, *expected, "{trigger:?} / {name}");
                assert_eq!(sim.count(Op::RemoveDomain), *removes, "{trigger:?} / {name}: removes only after a sign-out by choice");
                if wanted(world.facts) == Wanted::Present {
                    assert_ne!(sim.s.setup, Missing, "{trigger:?} / {name}: Missing is not a resting state while wanted is present");
                } else {
                    assert_eq!(sim.count(Op::AddDomain), 0, "{trigger:?} / {name}: never adds unless wanted");
                }
            }
        }
    }

    #[test]
    fn merged_triggers_produce_exactly_one_follow_up_check() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        for _ in 0..5 {
            sim.trigger(Trigger::TryAgain);
        }
        sim.trigger(Trigger::Launch);
        sim.run(&world);
        assert_eq!(sim.count(Op::Observe), 2, "six triggers during one check → exactly one follow-up");
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn sign_out_cancels_the_check_in_flight_removes_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        sim.step_op(&world); // StartEngine
        sim.step_op(&world); // AddDomain
        sim.trigger(Trigger::SignOut);
        sim.run(&world);
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::RemoveDomain]);
        assert_eq!(sim.s.setup, Missing);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(true)));
        // Held: nothing re-adds, whatever else arrives.
        sim.trigger(Trigger::TryAgain);
        sim.feed(Input::AppActivated);
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Idle);
        // Keys arriving again lift the hold.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn repair_removes_then_checks_exactly_once() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.ops.clear();
        sim.trigger(Trigger::Repair);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds(),
            vec![Op::RemoveDomain, Op::Observe, Op::StartEngine, Op::AddDomain, Op::ReadDomain, WAIT_10, Op::FinishReady]
        );
        assert_eq!(sim.s.setup, Ready);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
    }

    #[test]
    fn lock_cancels_the_check_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe → Adding
        assert_eq!(sim.s.setup, Adding);
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Missing);
        assert_eq!(sim.next(), Next::Idle, "no StartEngine after a lock");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.next(), Next::Idle);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }

    /// §7 on an injected clock: transient failures → exactly 4 attempts at 0/5/15/45 s.
    #[test]
    fn transient_failures_make_four_attempts_at_0_5_15_45() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { add: Err(fp(-2001)), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(sim.attempt_starts(), vec![0, 5, 15, 45]);
        assert_eq!(sim.s.setup, Failed);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::ExtensionLoading));
        assert_eq!((sim.s.attempt, sim.s.max_attempts), (4, 4));
        assert_eq!(sim.count(Op::StopEngine), 1, "the engine this check started is stopped once");
        // The person saw Adding throughout: one Adding publish, then one Failed.
        let lands: Vec<FinderSetup> = sim.fx.iter().filter_map(|e| match e { Effect::Transition(t) if t.from != t.to => Some(t.to), _ => None }).collect();
        assert_eq!(lands, vec![Adding, Failed]);
    }

    #[test]
    fn a_slow_attempt_delays_the_next_start() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { stable: Err(stabilization_timeout()), wait_secs: 10, ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(sim.attempt_starts(), vec![0, 10, 20, 45]);
        assert_eq!(sim.secs(), 55, "ends within about 55 s");
        assert_eq!(sim.s.reason, Some(FinderFailureReason::Timeout));
    }

    #[test]
    fn unknown_makes_two_attempts_and_not_retryable_reasons_make_one() {
        let mut unknown = Sim::new(Applications);
        unknown.trigger(Trigger::KeysArrived);
        unknown.run(&World { add: Err(FpError::new("SomeDomain", 1, "x")), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(unknown.attempt_starts(), vec![0, 5]);
        assert_eq!((unknown.s.setup, unknown.s.reason), (Failed, Some(FinderFailureReason::Unknown)));

        let taken = FpError::new(COCOA_DOMAIN, 516, "exists");
        let mut folder = Sim::new(Applications);
        folder.trigger(Trigger::KeysArrived);
        folder.run(&World { add: Err(taken.clone()), ..World::ok(SIGNED_IN, NotRegistered) });
        assert_eq!(folder.attempt_starts(), vec![0]);
        assert_eq!((folder.s.setup, folder.s.reason), (Failed, Some(FinderFailureReason::FolderTaken)));
        assert!(folder.fx.contains(&Effect::PersistFailure(Some((FinderFailureReason::FolderTaken, taken)))));
    }

    #[test]
    fn nothing_retries_after_failed_without_a_trigger_and_try_again_restarts_the_budget() {
        let world = World { add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")), ..World::ok(SIGNED_IN, NotRegistered) };
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Failed);
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle, "bringing the app forward does not retry");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.s.attempt, 1, "a fresh budget");
        sim.run(&world);
        assert_eq!(sim.count(Op::AddDomain), 2);
    }

    #[test]
    fn user_disabled_poll_reads_only_while_visible_and_a_flip_runs_one_check() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.s.setup, UserDisabled);
        let three = Duration::from_secs(3);
        assert_eq!(sim.next(), Next::WakeAt(sim.t0 + three));
        sim.now = sim.t0 + three;
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(sim.next(), Next::WakeAt(sim.t0 + three * 2), "no read while no window is visible");
        sim.now = sim.t0 + three * 2;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(sim.s.setup, UserDisabled);
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain), "once when the app becomes active");
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.count(Op::Observe), 2, "the flip runs exactly one check");
        assert_eq!(sim.count(Op::AddDomain), 0, "the poll never adds; a registered, enabled domain is only confirmed (§5.5 step 3)");
    }

    #[test]
    fn a_disk_image_launch_reports_the_reason_before_any_check_and_never_adds() {
        let mut sim = Sim::new(DiskImage);
        assert_eq!((sim.s.setup, sim.s.reason), (Missing, Some(FinderFailureReason::NotInApplications)));
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!((sim.s.setup, sim.s.reason), (Failed, Some(FinderFailureReason::NotInApplications)));
    }

    #[test]
    fn a_registered_domain_is_confirmed_even_from_a_disk_image() {
        // §5.5: the launch-location check applies only when the domain is not registered.
        let mut sim = Sim::new(DiskImage);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn ready_clears_a_persisted_failure() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World { add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")), ..World::ok(SIGNED_IN, NotRegistered) });
        sim.fx.clear();
        sim.trigger(Trigger::TryAgain);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!((sim.s.setup, sim.s.reason, sim.s.last_error.clone()), (Ready, None, None));
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    #[test]
    fn sign_in_after_a_sign_out_by_choice_clears_the_persisted_intent() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SessionFacts { signed_out_by_choice: true, ..SIGNED_IN }, NotRegistered));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(false)));
    }
}
```

Add `pub mod core;` to `finder_setup/mod.rs`.

- [ ] **Step 2: Run RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup::core > $EVID/t3-red.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t3-red.log
```

Expected: `rc=101`, `test result: FAILED. 0 passed; 21 failed` (`not yet implemented: Task 3 step 3`). Paste it into Notes.

- [ ] **Step 3: Implement `step`, `next_op` and the helpers**

Replace the two `todo!()` functions with the code below. It all goes in `core.rs`, above `#[cfg(test)]`.

```rust
pub fn step(mut state: CoreState, input: Input, now: Instant, policy: &RetryPolicy) -> (CoreState, Vec<Effect>) {
    let mut fx = Vec::new();
    match input {
        Input::Trigger(trigger) => on_trigger(&mut state, trigger, now, &mut fx),
        Input::Tick { window_visible } => on_tick(&mut state, window_visible, now, policy),
        Input::AppActivated => {
            if state.phase == Phase::Idle && state.setup == FinderSetup::UserDisabled {
                state.poll_pending = true;
            }
        }
        Input::Done(result) => {
            state.awaiting = false;
            on_done(&mut state, result, now, policy, &mut fx);
        }
    }
    (state, fx)
}

pub fn next_op(state: &mut CoreState, now: Instant, policy: &RetryPolicy) -> Next {
    if state.awaiting {
        tracing::warn!("finder setup: asked for the next operation while one is running");
        return Next::Idle;
    }
    let op = match state.phase {
        Phase::Removing { .. } => Op::RemoveDomain,
        Phase::Running(step) => match step {
            Step::Observe => Op::Observe,
            Step::StartEngine { .. } => Op::StartEngine,
            Step::Add => Op::AddDomain,
            Step::ReadAfterAdd => Op::ReadDomain,
            Step::Wait(Wait::AfterAdd) => Op::WaitStable(policy.stabilize_after_add),
            Step::Wait(Wait::ConfirmExisting) => Op::WaitStable(policy.confirm_existing),
            Step::Finish => Op::FinishReady,
            Step::StopEngine(_) => Op::StopEngine,
        },
        Phase::Backoff(until) => {
            if now < until {
                return Next::WakeAt(until);
            }
            state.attempt = state.attempt.saturating_add(1);
            state.phase = Phase::Running(Step::Observe);
            Op::Observe
        }
        Phase::Idle => {
            if state.poll_pending {
                state.poll_pending = false;
                Op::ReadDomain
            } else if state.setup == FinderSetup::UserDisabled {
                return state.poll_due.map_or(Next::Idle, Next::WakeAt);
            } else {
                return Next::Idle;
            }
        }
    };
    state.awaiting = true;
    Next::Run(op)
}

fn on_trigger(s: &mut CoreState, trigger: Trigger, now: Instant, fx: &mut Vec<Effect>) {
    fx.push(Effect::TriggerReceived(trigger));
    match trigger {
        Trigger::SignOut => {
            s.held = true;
            s.check_again = false;
            s.check_started_engine = false; // clear_session_impl stops the engine itself
            fx.push(Effect::PersistSignedOutByChoice(true));
            s.phase = Phase::Removing { requested: true, then_check: false };
        }
        Trigger::Repair => {
            s.check_again = false;
            s.check_started_engine = false; // reset_macos_integration stops the engine itself
            s.phase = Phase::Removing { requested: true, then_check: true };
        }
        Trigger::Lock => {
            s.held = true;
            s.check_again = false;
            if matches!(s.phase, Phase::Running(_) | Phase::Backoff(_)) {
                // No operation is in flight (the driver feeds events only between operations);
                // lock_vault stops any engine this check started right after this returns.
                s.phase = Phase::Idle;
                s.check_started = None;
                s.check_started_engine = false;
                if s.setup == FinderSetup::Adding {
                    let reason = missing_reason(s.launch);
                    set_state(s, FinderSetup::Missing, reason, None, fx);
                }
            }
        }
        Trigger::Launch | Trigger::KeysArrived => {
            s.held = false;
            request_check(s, now);
        }
        Trigger::TryAgain | Trigger::UserEnabledFlipped => {
            if !s.held {
                request_check(s, now);
            }
        }
    }
}

fn on_tick(s: &mut CoreState, window_visible: bool, now: Instant, policy: &RetryPolicy) {
    // Backoff expiry needs nothing here: `next_op` compares `now` with the deadline.
    if s.phase != Phase::Idle || s.setup != FinderSetup::UserDisabled {
        return;
    }
    if s.poll_due.is_some_and(|due| now >= due) {
        s.poll_due = Some(now + policy.user_disabled_poll);
        if window_visible {
            s.poll_pending = true;
        }
    }
}

fn request_check(s: &mut CoreState, now: Instant) {
    if s.phase == Phase::Idle {
        start_check(s, now);
    } else {
        s.check_again = true;
    }
}

fn start_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Running(Step::Observe);
    s.check_started = Some(now);
    s.check_started_engine = false;
    s.attempt = 1;
    s.max_attempts = 0;
    s.poll_pending = false;
}

fn on_done(s: &mut CoreState, result: OpResult, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    match (s.phase, result) {
        (Phase::Idle, OpResult::Domain(read)) => {
            // The UserDisabled poll. It never adds; a flip to enabled runs exactly one check.
            if s.setup == FinderSetup::UserDisabled && read == Ok(DomainState::Enabled) {
                fx.push(Effect::TriggerReceived(Trigger::UserEnabledFlipped));
                if !s.held {
                    start_check(s, now);
                }
            }
        }
        (Phase::Removing { requested, then_check }, OpResult::Removed(result)) => {
            let error = result.as_ref().err().cloned();
            let reason = missing_reason(s.launch);
            s.phase = Phase::Idle;
            s.check_started = None;
            set_state(s, FinderSetup::Missing, reason, error, fx);
            if requested {
                fx.push(Effect::RemoveFinished(result));
            }
            if (then_check || s.check_again) && !s.held {
                s.check_again = false;
                start_check(s, now);
            }
        }
        (Phase::Running(step), result) => on_check_result(s, step, result, now, policy, fx),
        (phase, result) => tracing::warn!(?phase, ?result, "finder setup: unexpected result, ignored"),
    }
}

fn on_check_result(s: &mut CoreState, step: Step, result: OpResult, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    match (step, result) {
        (Step::Observe, OpResult::Observed(observation)) => on_observed(s, observation, now, policy, fx),
        (Step::StartEngine { add }, OpResult::EngineStarted(Ok(newly_started))) => {
            s.check_started_engine |= newly_started;
            s.phase = Phase::Running(if add { Step::Add } else { Step::Wait(Wait::ConfirmExisting) });
        }
        (Step::Add, OpResult::Added(Ok(()))) => s.phase = Phase::Running(Step::ReadAfterAdd),
        (Step::ReadAfterAdd, OpResult::Domain(Ok(DomainState::Disabled))) => {
            s.reason = Some(FinderFailureReason::UserDisabled);
            end_check(s, Landing::UserDisabled, now, policy, fx);
        }
        // Enabled, not listed yet, or a failed read: wait for stabilization. A failed read is
        // never evidence of anything (the 1524 rule `decide_install_step` used to pin).
        (Step::ReadAfterAdd, OpResult::Domain(_)) => s.phase = Phase::Running(Step::Wait(Wait::AfterAdd)),
        (Step::Wait(_), OpResult::Stable(Ok(()))) => s.phase = Phase::Running(Step::Finish),
        (Step::Finish, OpResult::Finished(Ok(()))) => land_ready(s, now, fx),
        (Step::StopEngine(landing), OpResult::EngineStopped) => {
            s.check_started_engine = false;
            land_terminal(s, landing, now, policy, fx);
        }
        (
            _,
            OpResult::EngineStarted(Err(error))
            | OpResult::Added(Err(error))
            | OpResult::Stable(Err(error))
            | OpResult::Finished(Err(error)),
        ) => attempt_failed(s, error, now, policy, fx),
        (step, result) => tracing::warn!(?step, ?result, "finder setup: unexpected result, ignored"),
    }
}

fn on_observed(s: &mut CoreState, observation: Observation, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    let Observation { facts, domain } = observation;
    let wanted = wanted(facts);
    if wanted == Wanted::Present && facts.signed_out_by_choice {
        fx.push(Effect::PersistSignedOutByChoice(false));
    }
    match (wanted, domain) {
        (Wanted::Absent, Ok(DomainState::Enabled | DomainState::Disabled)) => {
            s.phase = Phase::Removing { requested: false, then_check: false };
        }
        (Wanted::Absent, Err(error)) => land_missing(s, Some(error), now, fx),
        (Wanted::Absent, Ok(DomainState::NotRegistered)) | (Wanted::NoAction, _) => land_missing(s, None, now, fx),
        (Wanted::Present, Ok(DomainState::Disabled)) => {
            s.reason = Some(FinderFailureReason::UserDisabled);
            end_check(s, Landing::UserDisabled, now, policy, fx);
        }
        (Wanted::Present, Ok(DomainState::Enabled)) => {
            if !matches!(s.setup, FinderSetup::Ready | FinderSetup::Adding) {
                set_state(s, FinderSetup::Adding, None, None, fx);
            }
            s.phase = Phase::Running(Step::StartEngine { add: false });
        }
        (Wanted::Present, Ok(DomainState::NotRegistered)) => {
            if s.launch.allows_add() {
                if s.setup != FinderSetup::Adding {
                    set_state(s, FinderSetup::Adding, None, None, fx);
                }
                s.phase = Phase::Running(Step::StartEngine { add: true });
            } else {
                s.reason = Some(FinderFailureReason::NotInApplications);
                s.max_attempts = 1;
                s.last_error = Some(FpError::app(
                    app_code::LAUNCH_LOCATION,
                    "Beebeeb was opened from a disk image or a translocated copy",
                ));
                end_check(s, Landing::Failed, now, policy, fx);
            }
        }
        (Wanted::Present, Err(error)) => attempt_failed(s, error, now, policy, fx),
    }
}

fn attempt_failed(s: &mut CoreState, error: FpError, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    let reason = policy::classify(&error);
    s.reason = Some(reason);
    s.max_attempts = policy.max_attempts(reason);
    s.last_error = Some(error.clone());
    if reason == FinderFailureReason::UserDisabled {
        return end_check(s, Landing::UserDisabled, now, policy, fx);
    }
    let started = s.check_started.unwrap_or(now);
    match policy.next_attempt_at(reason, s.attempt, started) {
        Some(at) => {
            // Adding → Adding: a silent retry, logged with its reason and attempt n of N.
            fx.push(Effect::Transition(Transition {
                from: s.setup,
                to: FinderSetup::Adding,
                reason: Some(reason),
                error: Some(error),
                attempt: s.attempt,
                max_attempts: s.max_attempts,
            }));
            s.setup = FinderSetup::Adding;
            fx.push(Effect::Publish);
            s.phase = Phase::Backoff(at.max(now));
        }
        None => end_check(s, Landing::Failed, now, policy, fx),
    }
}

/// Stop the engine this check started (if any), then land.
fn end_check(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    if s.check_started_engine {
        s.phase = Phase::Running(Step::StopEngine(landing));
    } else {
        land_terminal(s, landing, now, policy, fx);
    }
}

fn land_terminal(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    match landing {
        Landing::Failed => {
            let reason = s.reason.unwrap_or(FinderFailureReason::Unknown);
            let error = s
                .last_error
                .clone()
                .unwrap_or_else(|| FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "failed without an error"));
            fx.push(Effect::PersistFailure(Some((reason, error.clone()))));
            set_state(s, FinderSetup::Failed, Some(reason), Some(error), fx);
        }
        Landing::UserDisabled => {
            let error = s.last_error.clone();
            set_state(s, FinderSetup::UserDisabled, Some(FinderFailureReason::UserDisabled), error, fx);
            s.poll_due = Some(now + policy.user_disabled_poll);
        }
    }
    finish_check(s, now);
}

fn land_ready(s: &mut CoreState, now: Instant, fx: &mut Vec<Effect>) {
    s.last_error = None;
    s.check_started_engine = false; // the engine now belongs to the signed-in session
    fx.push(Effect::PersistFailure(None));
    set_state(s, FinderSetup::Ready, None, None, fx);
    finish_check(s, now);
}

fn land_missing(s: &mut CoreState, error: Option<FpError>, now: Instant, fx: &mut Vec<Effect>) {
    let reason = missing_reason(s.launch);
    set_state(s, FinderSetup::Missing, reason, error, fx);
    finish_check(s, now);
}

fn finish_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Idle;
    s.check_started = None;
    if s.check_again && !s.held {
        s.check_again = false;
        start_check(s, now);
    }
}

/// Every transition writes one log line and publishes once (§5.5 step 6).
fn set_state(s: &mut CoreState, to: FinderSetup, reason: Option<FinderFailureReason>, error: Option<FpError>, fx: &mut Vec<Effect>) {
    if s.setup == to && s.reason == reason {
        return;
    }
    fx.push(Effect::Transition(Transition {
        from: s.setup,
        to,
        reason,
        error,
        attempt: s.attempt,
        max_attempts: s.max_attempts.max(s.attempt),
    }));
    s.setup = to;
    s.reason = reason;
    fx.push(Effect::Publish);
}
```

- [ ] **Step 4: Run GREEN**

Same command as step 2 into `$EVID/t3-green.log`. Expected: `test result: ok. 21 passed; 0 failed`. If a test fails, fix the code, not the test. If you believe a test is wrong, stop and write down why in Notes for the lead. Do not edit the test.

- [ ] **Step 5: Mutation checks**

1. In `on_trigger`, delete `s.held = true;` from the `SignOut` arm. Expected failure: `sign_out_cancels_the_check_in_flight_removes_and_holds_until_keys_arrive` (the `Next::Idle` assertion). Restore it.
2. In `RetryPolicy::default` (Task 1), change `s(45)` to `s(30)`. Expected failures: `transient_failures_make_four_attempts_at_0_5_15_45`, `a_slow_attempt_delays_the_next_start` and Task 1's `retry_classes_follow_section_7`. Restore it.

Run both with the step-2 command into `$EVID/t3-mutation-1.log` and `-2.log`. Paste the failing test names into Notes, restore, rerun green.

- [ ] **Step 6: Check and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t3-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/finder_setup/core.rs
git commit -m "finder setup: the reconciler core, a pure state machine (spec A §5)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/finder_setup/core.rs src-tauri/src/finder_setup/mod.rs
git show --stat HEAD
```

---

## Task 4: The bridge returns structured errors (ObjC + Rust FFI)

**Lane R. macOS only**: `macos_file_provider` is `#[cfg(target_os = "macos")]` (lib.rs:63), so these tests run on the Mac gate and not on Linux CI. The lead notes that in the PR.

The struct shape follows the lead's suggestion, with one refinement: fixed-width integer fields (`int64_t`/`int32_t`), so the C and Rust layouts cannot drift on `long`. The size is pinned on both sides (a C `_Static_assert`, a Rust `const` assert) and compared **at runtime** through a C function, so a one-sided edit fails a test and not only a compile.

**Files:**
- Modify: `src-tauri/macos/FileProviderBridge.m` (whole file below)
- Modify: `src-tauri/src/macos_file_provider.rs`
- Modify: `src-tauri/src/lib.rs`: `remove_file_provider_domain` (macOS, ~line 2751) and `file_provider_visible_location` (macOS, ~line 2790). Both map `FpError` to `String` for their unchanged callers.

**Interfaces:**
- Consumes (Tasks 1, 3): `FpError`, `FpErrorCode`, `APP_DOMAIN`, `app_code::UNEXPECTED_BRIDGE_RETURN`, `core::DomainState`.
- Produces (used by Task 8): in `crate::macos_file_provider`:
  - `pub fn domain_state() -> Result<DomainState, FpError>`
  - `pub fn add_domain() -> Result<(), FpError>`
  - `pub fn wait_for_domain_ready(timeout: Duration) -> Result<(), FpError>`
  - `pub fn remove() -> Result<(), FpError>`
  - `pub fn visible_url() -> Result<Option<String>, FpError>`
  - `pub fn signal_working_set() -> Result<WorkingSetSignalOutcome, FpError>`
  - `pub fn cleanup_stale_domains() -> Result<StaleDomainCleanup, FpError>`
  - **Kept until Task 8**, with unchanged String signatures: `status()`, `install()`, `domain_user_enabled()`, `decide_install_step`, `status_outcome_from`, and `pub type DomainUserEnabledState = DomainState`.
- `runner.rs:1697` needs **no** change: it logs `error = %e`, and `FpError: Display`.

- [ ] **Step 1: Write the bridge tests first (Rust side)**

At the end of `src-tauri/src/macos_file_provider.rs`, add a second test module:

```rust
/// Spec §13.1 "Bridge": a real NSError built in Objective-C crosses the FFI and Rust
/// receives its domain and code unchanged.
#[cfg(test)]
mod bridge_error_tests {
    use super::*;
    use crate::finder_setup::error::{APP_DOMAIN, BRIDGE_DOMAIN, FpErrorCode, app_code, bridge_code};
    use std::ffi::CString;

    unsafe extern "C" {
        fn beebeeb_fp_test_fill_error(
            domain: *const c_char,
            code: i64,
            message: *const c_char,
            underlying_domain: *const c_char,
            underlying_code: i64,
            out_error: *mut BeebeebFpErrorC,
        );
        fn beebeeb_fp_test_fill_bridge_error(code: i64, message: *const c_char, out_error: *mut BeebeebFpErrorC);
        fn beebeeb_fp_test_error_size() -> usize;
    }

    fn through_bridge(domain: &str, code: i64, message: &str, underlying: Option<(&str, i64)>) -> FpError {
        let domain = CString::new(domain).unwrap();
        let message = CString::new(message).unwrap();
        let underlying_domain = underlying.map(|(d, _)| CString::new(d).unwrap());
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe {
            beebeeb_fp_test_fill_error(
                domain.as_ptr(),
                code,
                message.as_ptr(),
                underlying_domain.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                underlying.map_or(0, |(_, c)| c),
                &mut out,
            );
        }
        out.into_fp_error()
    }

    #[test]
    fn the_error_struct_has_the_same_size_on_both_sides() {
        assert_eq!(std::mem::size_of::<BeebeebFpErrorC>(), 1304);
        assert_eq!(unsafe { beebeeb_fp_test_error_size() }, std::mem::size_of::<BeebeebFpErrorC>());
    }

    #[test]
    fn every_classified_domain_and_code_crosses_the_bridge_unchanged() {
        let cases: &[(&str, i64)] = &[
            ("NSFileProviderErrorDomain", -2001),
            ("NSFileProviderErrorDomain", -2002),
            ("NSFileProviderErrorDomain", -2003),
            ("NSFileProviderErrorDomain", -2004),
            ("NSFileProviderErrorDomain", -2005),
            ("NSFileProviderErrorDomain", -2011),
            ("NSFileProviderErrorDomain", -2012),
            ("NSFileProviderErrorDomain", -2013),
            ("NSFileProviderErrorDomain", -2014),
            ("NSCocoaErrorDomain", 516),
            ("NSPOSIXErrorDomain", 17),
        ];
        for (domain, code) in cases {
            let error = through_bridge(domain, *code, "fixture", None);
            assert_eq!((error.domain.as_str(), error.code), (*domain, *code));
            assert_eq!(error.message, "fixture");
            assert_eq!(error.underlying, None, "{domain} {code}");
        }
    }

    #[test]
    fn the_underlying_error_crosses_with_its_own_domain_and_code() {
        let error = through_bridge("NSCocoaErrorDomain", 516, "exists", Some(("NSPOSIXErrorDomain", 17)));
        assert_eq!(error.underlying, Some(FpErrorCode { domain: "NSPOSIXErrorDomain".into(), code: 17 }));
    }

    #[test]
    fn bridge_synthesized_errors_carry_the_bridge_domain() {
        let message = CString::new("Timed out waiting for the Beebeeb File Provider domain to become available").unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_bridge_error(bridge_code::STABILIZATION_TIMEOUT, message.as_ptr(), &mut out) };
        let error = out.into_fp_error();
        assert_eq!((error.domain.as_str(), error.code), (BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT));
        assert_eq!(error.underlying, None);
    }

    #[test]
    fn a_long_message_is_cut_not_overflowed() {
        // 6000 UTF-8 bytes into a 1024-byte field: strlcpy may cut inside a code point, which
        // from_utf8_lossy turns into U+FFFD, so count chars, not bytes.
        let error = through_bridge("NSCocoaErrorDomain", 1, &"é".repeat(3000), None);
        assert!(error.message.chars().count() <= 1023, "{}", error.message.chars().count());
        assert_eq!(error.domain, "NSCocoaErrorDomain");
    }

    #[test]
    fn an_unfilled_error_becomes_an_app_error_not_an_empty_one() {
        let error = BeebeebFpErrorC::zeroed().into_fp_error();
        assert_eq!((error.domain.as_str(), error.code), (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN));
    }
}
```

- [ ] **Step 2: Add the Rust mirror and run RED**

At the top of `macos_file_provider.rs`, below the existing `use` lines, add the struct and its conversion. Leave the existing FFI as it is for now.

```rust
use std::time::Duration;

use crate::finder_setup::core::DomainState;
use crate::finder_setup::error::{FpError, FpErrorCode, app_code};

/// Mirror of `BeebeebFpError` in `src-tauri/macos/FileProviderBridge.m` (spec §6.1). The C side
/// `_Static_assert`s the same size, and `beebeeb_fp_test_error_size` is compared at runtime.
#[repr(C)]
pub struct BeebeebFpErrorC {
    pub code: i64,
    pub underlying_code: i64,
    pub has_underlying: i32,
    pub domain: [c_char; 128],
    pub underlying_domain: [c_char; 128],
    pub message: [c_char; 1024],
}

const _: () = assert!(std::mem::size_of::<BeebeebFpErrorC>() == 1304);

impl BeebeebFpErrorC {
    pub fn zeroed() -> Self {
        Self { code: 0, underlying_code: 0, has_underlying: 0, domain: [0; 128], underlying_domain: [0; 128], message: [0; 1024] }
    }

    pub fn into_fp_error(self) -> FpError {
        let domain = c_array_to_string(&self.domain);
        if domain.is_empty() {
            return FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "the File Provider bridge failed without describing the error");
        }
        FpError {
            domain,
            code: self.code,
            message: c_array_to_string(&self.message),
            underlying: (self.has_underlying != 0).then(|| FpErrorCode {
                domain: c_array_to_string(&self.underlying_domain),
                code: self.underlying_code,
            }),
        }
    }
}

/// Bounded read: never past the array, even if the C side forgot the NUL.
fn c_array_to_string(array: &[c_char]) -> String {
    let bytes: Vec<u8> = array.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}
```

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib macos_file_provider > $EVID/t4-red.log 2>&1; echo "rc=$?"; grep -E "Undefined symbols|_beebeeb_fp_test|error:" $EVID/t4-red.log | head
```

Expected: `rc=101` with a link error, `Undefined symbols … _beebeeb_fp_test_fill_error` (and the other two test symbols). Paste it into Notes.

- [ ] **Step 3: Replace `src-tauri/macos/FileProviderBridge.m`**

Write the whole file:

```objc
#import <Foundation/Foundation.h>
#import <FileProvider/FileProvider.h>
#import <dispatch/dispatch.h>
#include <stdint.h>
#include <string.h>

static NSString *BeebeebDomainIdentifier = @"io.beebeeb.app.domain";
// Task 1698 part 3 (1696 G8): the domain's display name is "Beebeeb" — the
// app's name and what the system DB + Finder sidebar already show. The old
// "Drive" was the zombie domain's residue (1696 forensics: we registered
// "Drive" while the system showed "Beebeeb"). Which side wins on an existing
// registration is device-verifiable only (addDomain updates the stored
// domain); the registered truth is now the app's own name.
//
// Correction — 2026-10-06 (spec docs/specs/2026-10-06-macos-finder-setup-reconciler.md §2):
// the forensics line above read the ZOMBIE's name. "Beebeeb" was the display name of the
// orphan `io.beebeeb.desktop.FileProvider` domain, whose folder is
// ~/Library/CloudStorage/Beebeeb-Beebeeb. Renaming ours to "Beebeeb" made ours ask for that
// same folder, and every add then failed with the folder collision. The name stays "Beebeeb"
// (the orphan is cleared by spec §11's one-off helper); this note keeps the wrong claim visible.
static NSString *BeebeebDomainDisplayName = @"Beebeeb";

// Spec 2026-10-06 §6.1: every failure crosses the FFI as (domain, code, message) plus the
// NSUnderlyingErrorKey error's domain and code (the folder collision may arrive as an
// underlying POSIX EEXIST). Mirrored by `BeebeebFpErrorC` in src-tauri/src/macos_file_provider.rs.
typedef struct {
    int64_t code;
    int64_t underlying_code;
    int32_t has_underlying;
    char domain[128];
    char underlying_domain[128];
    char message[1024];
} BeebeebFpError;

_Static_assert(sizeof(BeebeebFpError) == 1304, "BeebeebFpError changed: update BeebeebFpErrorC in macos_file_provider.rs");

// Errors this bridge synthesizes when no NSError exists. Codes mirrored by
// `finder_setup::error::bridge_code` in Rust.
static NSString *const BeebeebBridgeErrorDomain = @"io.beebeeb.bridge";
enum {
    BeebeebBridgeManagerUnavailable = 1,
    BeebeebBridgeStabilizationTimeout = 2,
    BeebeebBridgeResolveUrlTimeout = 3,
    BeebeebBridgeSignalTimeout = 4,
    BeebeebBridgeNoIdentifier = 5,
};

static void BeebeebCopyString(NSString *value, char *buffer, size_t buffer_len) {
    if (buffer == NULL || buffer_len == 0) {
        return;
    }
    const char *utf8 = [value UTF8String];
    strlcpy(buffer, utf8 ?: "", buffer_len);
}

static void BeebeebFillError(NSError *error, BeebeebFpError *out) {
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));
    out->code = (int64_t)error.code;
    BeebeebCopyString(error.domain ?: @"unknown-domain", out->domain, sizeof(out->domain));
    BeebeebCopyString(error.localizedDescription ?: @"unknown File Provider error", out->message, sizeof(out->message));
    NSError *underlying = error.userInfo[NSUnderlyingErrorKey];
    if ([underlying isKindOfClass:[NSError class]]) {
        out->has_underlying = 1;
        out->underlying_code = (int64_t)underlying.code;
        BeebeebCopyString(underlying.domain ?: @"unknown-domain", out->underlying_domain, sizeof(out->underlying_domain));
    }
}

static void BeebeebFillBridgeError(int64_t code, NSString *message, BeebeebFpError *out) {
    if (out == NULL) {
        return;
    }
    memset(out, 0, sizeof(*out));
    out->code = code;
    BeebeebCopyString(BeebeebBridgeErrorDomain, out->domain, sizeof(out->domain));
    BeebeebCopyString(message, out->message, sizeof(out->message));
}

static NSFileProviderDomain *BeebeebDomain(void) {
    NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:BeebeebDomainIdentifier
                                                            displayName:BeebeebDomainDisplayName];
    // Task 1698 part 2 (ruling: full trash sync): macOS 13+ surfaces the
    // system trash container as Finder's Trash when this is YES. It IS the
    // default, but it is set explicitly so the ruling is visible in code and
    // survives any future default change.
    if (@available(macOS 13.0, *)) {
        domain.supportsSyncingTrash = YES;
    }
    return domain;
}

// The system's live registry. Returns 0 and fills `*found` (nil when not registered), or -1.
static int BeebeebFindOurDomain(NSFileProviderDomain **found, BeebeebFpError *out_error) {
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSArray<NSFileProviderDomain *> *found_domains = nil;
    __block NSError *found_error = nil;
    [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
        found_domains = domains;
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    *found = nil;
    for (NSFileProviderDomain *domain in found_domains) {
        if ([domain.identifier isEqualToString:BeebeebDomainIdentifier]) {
            *found = domain;
            break;
        }
    }
    return 0;
}

static int BeebeebWaitForDomainReady(NSFileProviderDomain *domain, double timeout_seconds, BeebeebFpError *out_error) {
    NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:domain];
    if (manager == nil) {
        BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable,
                               @"File Provider manager is unavailable for the Beebeeb domain", out_error);
        return -1;
    }
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    __block NSError *found_error = nil;
    [manager waitForStabilizationWithCompletionHandler:^(NSError *error) {
        found_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    int64_t timeout_nanos = (int64_t)(timeout_seconds * (double)NSEC_PER_SEC);
    long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, timeout_nanos));
    if (wait_result != 0) {
        BeebeebFillBridgeError(BeebeebBridgeStabilizationTimeout,
                               @"Timed out waiting for the Beebeeb File Provider domain to become available", out_error);
        return -1;
    }
    if (found_error != nil) {
        BeebeebFillError(found_error, out_error);
        return -1;
    }
    return 0;
}

// Kept until plan Task 8 deletes `macos_file_provider::status()`.
int beebeeb_fp_status(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderDomain *ours = nil;
        if (BeebeebFindOurDomain(&ours, out_error) != 0) {
            return -1;
        }
        if (ours == nil) {
            return 0;
        }
        return BeebeebWaitForDomainReady(ours, 2.0, out_error) == 0 ? 1 : -1;
    }
}

int beebeeb_fp_visible_url(char *url_buffer, unsigned long url_buffer_len, BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            BeebeebFillBridgeError(BeebeebBridgeManagerUnavailable,
                                   @"File Provider manager is unavailable for the Beebeeb domain", out_error);
            return -1;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSURL *found_url = nil;
        __block NSError *found_error = nil;
        [manager getUserVisibleURLForItemIdentifier:NSFileProviderRootContainerItemIdentifier
                                  completionHandler:^(NSURL *url, NSError *error) {
            found_url = url;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeResolveUrlTimeout, @"Timed out resolving the Beebeeb Finder location", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        if (found_url == nil) {
            return 0;
        }
        BeebeebCopyString(found_url.path ?: found_url.absoluteString, url_buffer, url_buffer_len);
        return 1;
    }
}

// Issue 4 (task 1524): the install decision lives in Rust; this file exposes dumb,
// synchronous-blocking primitives. Since spec 2026-10-06 the decision is the Finder
// reconciler (src-tauri/src/finder_setup/core.rs).

// Kept until plan Task 8 deletes `macos_file_provider::install()`.
// Returns: 1 = domain exists, 0 = does not, -1 = lookup error (out_error set).
int beebeeb_fp_domain_exists(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderDomain *ours = nil;
        if (BeebeebFindOurDomain(&ours, out_error) != 0) {
            return -1;
        }
        return ours != nil ? 1 : 0;
    }
}

// Returns: 1 = userEnabled == YES, 0 = userEnabled == NO, 2 = domain not registered,
// -1 = lookup error (out_error set). `userEnabled` has shipped since macOS 11.0, below this
// bridge's 14.0 deployment target (`-mmacosx-version-min=14.0` in src-tauri/build.rs).
int beebeeb_fp_domain_user_enabled(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderDomain *ours = nil;
        if (BeebeebFindOurDomain(&ours, out_error) != 0) {
            return -1;
        }
        if (ours == nil) {
            return 2;
        }
        return ours.userEnabled ? 1 : 0;
    }
}

// `+[NSFileProviderManager addDomain:completionHandler:]` only; does not wait for
// stabilization. Returns: 0 = success, -1 = error (out_error set).
int beebeeb_fp_add_domain(BeebeebFpError *out_error) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [NSFileProviderManager addDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// Returns: 0 = stable, -1 = timeout or error (out_error set).
int beebeeb_fp_wait_for_domain_ready(double timeout_seconds, BeebeebFpError *out_error) {
    @autoreleasepool {
        return BeebeebWaitForDomainReady(BeebeebDomain(), timeout_seconds, out_error);
    }
}

int beebeeb_fp_remove(BeebeebFpError *out_error) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [NSFileProviderManager removeDomain:BeebeebDomain() completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// Task 1697: signal the replica's WORKING SET after the daemon applied an operation batch.
// Returns: 0 = signaled (or domain not registered yet — nothing to signal), -1 = error.
int beebeeb_fp_signal_working_set(BeebeebFpError *out_error) {
    @autoreleasepool {
        NSFileProviderManager *manager = [NSFileProviderManager managerForDomain:BeebeebDomain()];
        if (manager == nil) {
            // Not registered: nothing to signal is not a failure; the engine keeps running.
            return 0;
        }
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [manager signalEnumeratorForContainerItemIdentifier:NSFileProviderWorkingSetContainerItemIdentifier
                                          completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        long wait_result = dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
        if (wait_result != 0) {
            BeebeebFillBridgeError(BeebeebBridgeSignalTimeout, @"Timed out signaling the Beebeeb File Provider working set", out_error);
            return -1;
        }
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// Task 1698 part 3: every registered identifier of THIS provider, newline-separated.
// Returns the count, or -1 (out_error set).
int beebeeb_fp_list_domains(char *ids_buffer, unsigned long ids_buffer_len, BeebeebFpError *out_error) {
    @autoreleasepool {
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray<NSFileProviderDomain *> *found_domains = nil;
        __block NSError *found_error = nil;
        [NSFileProviderManager getDomainsWithCompletionHandler:^(NSArray<NSFileProviderDomain *> *domains, NSError *error) {
            found_domains = domains;
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        NSMutableString *joined = [NSMutableString string];
        for (NSFileProviderDomain *domain in found_domains) {
            if (joined.length > 0) {
                [joined appendString:@"\n"];
            }
            [joined appendString:domain.identifier ?: @""];
        }
        BeebeebCopyString(joined, ids_buffer, ids_buffer_len);
        return (int)found_domains.count;
    }
}

// Task 1698 part 3: remove ONE domain by identifier. Returns 0 = removed (or already gone), -1.
int beebeeb_fp_remove_domain_by_id(const char *identifier, BeebeebFpError *out_error) {
    @autoreleasepool {
        if (identifier == NULL) {
            BeebeebFillBridgeError(BeebeebBridgeNoIdentifier, @"no domain identifier given", out_error);
            return -1;
        }
        NSString *domain_id = [NSString stringWithUTF8String:identifier];
        NSFileProviderDomain *domain = [[NSFileProviderDomain alloc] initWithIdentifier:domain_id displayName:domain_id];
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSError *found_error = nil;
        [NSFileProviderManager removeDomain:domain completionHandler:^(NSError *error) {
            found_error = error;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER);
        if (found_error != nil) {
            BeebeebFillError(found_error, out_error);
            return -1;
        }
        return 0;
    }
}

// ── Test hooks (spec §13.1 "Bridge") ─────────────────────────────────────────
// Build a real NSError (with an underlying error when `underlying_domain` is non-NULL) and
// run it through the SAME BeebeebFillError every call above uses. Called only from Rust tests.
void beebeeb_fp_test_fill_error(const char *domain, int64_t code, const char *message,
                                const char *underlying_domain, int64_t underlying_code,
                                BeebeebFpError *out_error) {
    @autoreleasepool {
        NSMutableDictionary *info = [NSMutableDictionary dictionary];
        info[NSLocalizedDescriptionKey] = [NSString stringWithUTF8String:message ?: ""];
        if (underlying_domain != NULL) {
            info[NSUnderlyingErrorKey] = [NSError errorWithDomain:[NSString stringWithUTF8String:underlying_domain]
                                                             code:(NSInteger)underlying_code
                                                         userInfo:nil];
        }
        NSError *error = [NSError errorWithDomain:[NSString stringWithUTF8String:domain ?: ""]
                                             code:(NSInteger)code
                                         userInfo:info];
        BeebeebFillError(error, out_error);
    }
}

void beebeeb_fp_test_fill_bridge_error(int64_t code, const char *message, BeebeebFpError *out_error) {
    @autoreleasepool {
        BeebeebFillBridgeError(code, [NSString stringWithUTF8String:message ?: ""], out_error);
    }
}

unsigned long beebeeb_fp_test_error_size(void) {
    return sizeof(BeebeebFpError);
}
```

- [ ] **Step 4: Rewrite the Rust FFI layer of `macos_file_provider.rs`**

Replace the `unsafe extern "C"` blocks, `call_bridge`, `buffer_to_string` and the functions that used them, so the module reads as follows. Everything from `pub enum InstallOutcome` through `status_outcome_from` keeps its docs and logic. Only the lines shown change, and the existing `mod tests` stays as it is.

```rust
unsafe extern "C" {
    fn beebeeb_fp_status(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_visible_url(url_buffer: *mut c_char, url_buffer_len: usize, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_domain_exists(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_domain_user_enabled(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_add_domain(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_wait_for_domain_ready(timeout_seconds: f64, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_remove(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_signal_working_set(out_error: *mut BeebeebFpErrorC) -> i32;
}

/// Run one bridge primitive; a negative return carries the filled error.
fn call(function: unsafe extern "C" fn(*mut BeebeebFpErrorC) -> i32) -> Result<i32, FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { function(&mut out) };
    if code >= 0 { Ok(code) } else { Err(out.into_fp_error()) }
}

/// The pre-reconciler name; Task 8 deletes it with `install()`/`status()`.
pub type DomainUserEnabledState = DomainState;

const INSTALL_STABILIZATION_TIMEOUT: Duration = Duration::from_secs(10);

/// `NSFileProviderDomain.userEnabled` from the system's live registry.
pub fn domain_state() -> Result<DomainState, FpError> {
    match call(beebeeb_fp_domain_user_enabled)? {
        1 => Ok(DomainState::Enabled),
        0 => Ok(DomainState::Disabled),
        2 => Ok(DomainState::NotRegistered),
        other => Err(FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, format!("beebeeb_fp_domain_user_enabled returned {other}"))),
    }
}

pub fn add_domain() -> Result<(), FpError> {
    call(beebeeb_fp_add_domain).map(|_| ())
}

pub fn wait_for_domain_ready(timeout: Duration) -> Result<(), FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_wait_for_domain_ready(timeout.as_secs_f64(), &mut out) };
    if code < 0 { Err(out.into_fp_error()) } else { Ok(()) }
}

pub fn remove() -> Result<(), FpError> {
    call(beebeeb_fp_remove).map(|_| ())
}

pub fn visible_url() -> Result<Option<String>, FpError> {
    let mut url_buffer = [0 as c_char; 2048];
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_visible_url(url_buffer.as_mut_ptr(), url_buffer.len(), &mut out) };
    match code {
        c if c < 0 => Err(out.into_fp_error()),
        0 => Ok(None),
        _ => Ok(Some(c_array_to_string(&url_buffer)).filter(|s| !s.is_empty())),
    }
}

fn domain_exists() -> Result<bool, FpError> {
    Ok(call(beebeeb_fp_domain_exists)? == 1)
}

pub fn status() -> Result<StatusOutcome, String> {
    status_outcome_from(&domain_state().map_err(|e| e.to_string()), || {
        Ok(call(beebeeb_fp_status).map_err(|e| e.to_string())? == 1)
    })
}

/// Frontend-facing tri-state read (the `finder_domain_user_enabled` command).
pub fn domain_user_enabled() -> Result<DomainUserEnabledState, String> {
    domain_state().map_err(|e| e.to_string())
}

pub fn install() -> Result<InstallOutcome, String> {
    let existed_before_add = domain_exists().map_err(|e| e.to_string())?;
    if existed_before_add && decide_install_step(&domain_state().map_err(|e| e.to_string())) == InstallDecision::UserDisabled {
        return Ok(InstallOutcome::UserDisabled);
    }
    add_domain().map_err(|e| e.to_string())?;
    if decide_install_step(&domain_state().map_err(|e| e.to_string())) == InstallDecision::UserDisabled {
        return Ok(InstallOutcome::UserDisabled);
    }
    if let Err(setup_error) = wait_for_domain_ready(INSTALL_STABILIZATION_TIMEOUT) {
        let setup_error = setup_error.to_string();
        if !existed_before_add
            && let Err(cleanup_error) = remove()
        {
            return Err(format!("{setup_error}; cleanup failed: {cleanup_error}"));
        }
        return Err(setup_error);
    }
    Ok(InstallOutcome::Installed)
}

pub fn signal_working_set() -> Result<WorkingSetSignalOutcome, FpError> {
    match call(beebeeb_fp_signal_working_set)? {
        0 => Ok(WorkingSetSignalOutcome::Delivered),
        other => Err(FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, format!("beebeeb_fp_signal_working_set returned {other}"))),
    }
}
```

Delete the old `DomainUserEnabledState` enum (now the alias), the old `domain_user_enabled_state`, `INSTALL_STABILIZATION_TIMEOUT_SECONDS`, `call_bridge`, `buffer_to_string`, and the now-unused `use std::ffi::CStr;` at line 1. Change `decide_install_step` and `status_outcome_from` only in their type names: `Result<DomainUserEnabledState, String>` still compiles through the alias. Leave their bodies and the existing tests as they are.

In `mod cleanup_ffi`, the error parameters become `out_error: *mut super::BeebeebFpErrorC`, and the functions return `Result<_, FpError>`:

```rust
#[cfg(target_os = "macos")]
mod cleanup_ffi {
    use super::{BeebeebFpErrorC, c_array_to_string};
    use crate::finder_setup::error::FpError;
    use std::os::raw::c_char;

    unsafe extern "C" {
        fn beebeeb_fp_list_domains(ids_buffer: *mut c_char, ids_buffer_len: usize, out_error: *mut BeebeebFpErrorC) -> i32;
        fn beebeeb_fp_remove_domain_by_id(identifier: *const c_char, out_error: *mut BeebeebFpErrorC) -> i32;
    }

    pub fn list_domains() -> Result<Vec<String>, FpError> {
        let mut ids_buffer = [0 as c_char; 8192];
        let mut out = BeebeebFpErrorC::zeroed();
        let count = unsafe { beebeeb_fp_list_domains(ids_buffer.as_mut_ptr(), ids_buffer.len(), &mut out) };
        if count < 0 {
            return Err(out.into_fp_error());
        }
        Ok(c_array_to_string(&ids_buffer).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
    }

    pub fn remove_domain(identifier: &str) -> Result<(), FpError> {
        let c_id = std::ffi::CString::new(identifier)
            .map_err(|_| FpError::app(crate::finder_setup::error::app_code::UNEXPECTED_BRIDGE_RETURN, "domain identifier contained a NUL byte"))?;
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe { beebeeb_fp_remove_domain_by_id(c_id.as_ptr(), &mut out) };
        if code < 0 { Err(out.into_fp_error()) } else { Ok(()) }
    }
}
```

`StaleDomainCleanup.removals` becomes `Vec<(String, Result<(), FpError>)>`, and `cleanup_stale_domains()` returns `Result<StaleDomainCleanup, FpError>`. The body is unchanged. Delete the second `unsafe extern "C" { fn beebeeb_fp_signal_working_set … }` block that sat at line ~415. Its declaration is now in the first block.

Under the `DOMAIN_IDENTIFIER` doc comment (line ~248), keep the original text and append:

```rust
///
/// **Correction — 2026-10-06 (spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md` §2):**
/// the paragraph above was wrong on two counts. The zombie's folder is `Beebeeb-Beebeeb`, not
/// `Beebeeb-Drive`. And no signed `io.beebeeb.app` context can enumerate it, because
/// `NSFileProviderManager.getDomains` returns only the CALLING provider's domains, so the
/// sweep below can never see `io.beebeeb.desktop.FileProvider`. The orphan is cleared by the
/// one-off helper of spec §11. The 1696 forensics read the zombie's display name ("Beebeeb"),
/// and 1698 renamed our domain to match it, which created the folder collision.
```

In `src-tauri/src/lib.rs`, map the two String callers:

```rust
#[cfg(target_os = "macos")]
fn remove_file_provider_domain() -> Result<(), String> {
    macos_file_provider::remove().map_err(|e| e.to_string())
}
```

```rust
#[cfg(target_os = "macos")]
fn file_provider_visible_location() -> Result<Option<String>, String> {
    macos_file_provider::visible_url().map_err(|e| e.to_string())
}
```

`file_provider_domain_user_enabled` (lib.rs ~2804) still matches `macos_file_provider::DomainUserEnabledState::{Enabled, Disabled, NotRegistered}`, which compiles through the alias.

- [ ] **Step 5: Run GREEN**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib macos_file_provider > $EVID/t4-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t4-green.log
```

Expected: `test result: ok. N passed; 0 failed`, where N is the old module count (10) plus 6. Write N into Notes.

- [ ] **Step 6: Mutation check**

In `BeebeebFillError`, change `out->code = (int64_t)error.code;` to `out->code = 0;`. Rerun into `$EVID/t4-mutation.log`. Expected: `every_classified_domain_and_code_crosses_the_bridge_unchanged` and `the_underlying_error_crosses_…` fail (the second fails only if it asserts the code; it does, through `FpErrorCode`). Restore and rerun green.

- [ ] **Step 7: Check all targets and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t4-check.log 2>&1; echo "rc=$?"
cd $WT && git commit -m "file provider bridge: structured (domain, code, message, underlying) errors; 1698 correction (spec A §2, §6.1)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/macos/FileProviderBridge.m src-tauri/src/macos_file_provider.rs src-tauri/src/lib.rs
git show --stat HEAD
```

---

## Task 5: The lifecycle log

**Lane R.** It is placed before the driver (and not after it, as the lead's suggested order has it) because the driver writes a line on every transition. Building the log first means the driver never needs a temporary `tracing` shim.

**Files:**
- Create: `src-tauri/src/lifecycle_log.rs`
- Modify: `src-tauri/src/state_db.rs` (add `StateDb::known_names` next to `queue_diagnostics_with_paths`, ~line 2944, plus one test)
- Modify: `src-tauri/src/lib.rs` (module list: `#[allow(dead_code)] mod lifecycle_log;`, with the same comment as `finder_setup`)

**Interfaces:**
- Consumes: `diagnostic_redaction::{KnownNames, redact_for_export}`, `core::Trigger`, `FpError`, `LaunchLocation`, `FinderSetup`, `FinderFailureReason`.
- Produces (used by Tasks 7, 8, 9):
  - `lifecycle_log::{FILE_NAME = "lifecycle.log", MAX_BYTES = 1_048_576, KEEP_FILES = 3, TAIL_LINES = 200}`
  - `enum LifecycleEvent { Launch { app_version, macos_version, launch_location }, Trigger(Trigger), Transition { from, to, reason, error: Option<FpError>, attempt, max_attempts }, SignedIn, SignedOut, UpdateDownloaded { from, to }, UpdateInstalled { from, to } }`
  - `fn format_line(&LifecycleEvent, &KnownNames, SystemTime) -> String`, `pub(crate) fn token(&str) -> String`
  - `struct LifecycleLog { new(dir, max_bytes), path(), append_line(&str) -> io::Result<()>, tail(n) -> Vec<String> }`
  - `fn init(dir: PathBuf, names: fn() -> KnownNames)`, `fn default_dir() -> Option<PathBuf>`, `fn event(LifecycleEvent)`, `fn tail(n: usize) -> Vec<String>`
  - `StateDb::known_names(&self, extra_paths: &[String]) -> rusqlite::Result<KnownNames>`

- [ ] **Step 1: Write the module with tests, and `todo!()` in `format_line`, `append_line`, `tail` (both) and `event`**

```rust
//! The lifecycle log (spec 2026-10-06 §8). `<home>/Library/Logs/Beebeeb/lifecycle.log`. Under
//! the app sandbox `<home>` is the app container, so a shipped build writes
//! `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log` (plan "Spec
//! issues" 4).
//!
//! A closed vocabulary of typed events. The only free text is an NSError message, which is
//! passed through `diagnostic_redaction::redact_for_export`. It is NOT a `tracing` sink:
//! nothing from the general tracing stream (engine errors embed paths and file names) reaches
//! the file. It holds no user content, so it survives sign-out.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::diagnostic_redaction::{KnownNames, redact_for_export};
use crate::finder_setup::core::Trigger;
use crate::finder_setup::error::FpError;
use crate::finder_setup::launch_location::LaunchLocation;
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

pub const FILE_NAME: &str = "lifecycle.log";
pub const MAX_BYTES: u64 = 1024 * 1024;
/// `lifecycle.log`, `.1`, `.2`.
pub const KEEP_FILES: usize = 3;
/// What "Copy details" and the 1685 support bundle include.
pub const TAIL_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    Launch { app_version: String, macos_version: String, launch_location: LaunchLocation },
    Trigger(Trigger),
    Transition {
        from: FinderSetup,
        to: FinderSetup,
        reason: Option<FinderFailureReason>,
        error: Option<FpError>,
        attempt: u8,
        max_attempts: u8,
    },
    /// Keys arrived on this Mac. No email, no account id.
    SignedIn,
    SignedOut,
    UpdateDownloaded { from: String, to: String },
    UpdateInstalled { from: String, to: String },
}

/// One line, no newline: an RFC 3339 UTC timestamp to the second, then the event.
pub fn format_line(event: &LifecycleEvent, names: &KnownNames, at: SystemTime) -> String {
    todo!("Task 5 step 3")
}

/// Versions and NSError domains are ours or Apple's, but they still pass a whitelist, so no free
/// text can slip in: `[A-Za-z0-9._-]`, at most 64 characters.
pub(crate) fn token(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(64).collect()
}

pub struct LifecycleLog {
    dir: PathBuf,
    max_bytes: u64,
}

impl LifecycleLog {
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn rotated(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{FILE_NAME}.{n}"))
    }

    pub fn append_line(&self, line: &str) -> std::io::Result<()> {
        todo!("Task 5 step 3")
    }

    /// The last `n` lines across the rotated files, oldest first.
    pub fn tail(&self, n: usize) -> Vec<String> {
        todo!("Task 5 step 3")
    }
}

static GLOBAL: Mutex<Option<(LifecycleLog, fn() -> KnownNames)>> = Mutex::new(None);

/// Called once from `setup()` on macOS. Before `init` (tests, Windows, Linux) `event` is a no-op.
pub fn init(dir: PathBuf, names: fn() -> KnownNames) {
    *GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some((LifecycleLog::new(dir, MAX_BYTES), names));
}

/// `<home>/Library/Logs/Beebeeb`.
pub fn default_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library").join("Logs").join("Beebeeb"))
}

pub fn event(event: LifecycleEvent) {
    todo!("Task 5 step 3")
}

pub fn tail(n: usize) -> Vec<String> {
    todo!("Task 5 step 3")
}

/// Write one event to `log`. `event` does exactly this once `init` has run.
pub fn write_event(log: &LifecycleLog, event: &LifecycleEvent, names: &KnownNames, at: SystemTime) -> std::io::Result<()> {
    log.append_line(&format_line(event, names, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::COCOA_DOMAIN;
    use std::time::{Duration, UNIX_EPOCH};

    /// 2026-10-06T13:05:09Z, the moment Guus's 0.8.11 saved its last Finder failure.
    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_791_291_909)
    }

    fn names(paths: &[&str]) -> KnownNames {
        let mut known = KnownNames::new();
        for path in paths {
            known.add_path(path);
        }
        known.finish()
    }

    fn transition(message: &str) -> LifecycleEvent {
        LifecycleEvent::Transition {
            from: FinderSetup::Adding,
            to: FinderSetup::Failed,
            reason: Some(FinderFailureReason::FolderTaken),
            error: Some(FpError::new(COCOA_DOMAIN, 516, message).with_underlying("NSPOSIXErrorDomain", 17)),
            attempt: 1,
            max_attempts: 1,
        }
    }

    #[test]
    fn every_event_has_one_closed_line_shape() {
        let none = KnownNames::new();
        let line = |event: LifecycleEvent| format_line(&event, &none, at());
        assert_eq!(
            line(LifecycleEvent::Launch { app_version: "0.8.12".into(), macos_version: "26.0".into(), launch_location: LaunchLocation::Applications }),
            "2026-10-06T13:05:09Z launch app=0.8.12 macos=26.0 location=applications"
        );
        assert_eq!(line(LifecycleEvent::Trigger(Trigger::KeysArrived)), "2026-10-06T13:05:09Z trigger keys_arrived");
        assert_eq!(line(LifecycleEvent::SignedIn), "2026-10-06T13:05:09Z signed_in");
        assert_eq!(line(LifecycleEvent::SignedOut), "2026-10-06T13:05:09Z signed_out");
        assert_eq!(
            line(LifecycleEvent::UpdateDownloaded { from: "0.8.11".into(), to: "0.8.12-alpha.1".into() }),
            "2026-10-06T13:05:09Z update_downloaded from=0.8.11 to=0.8.12-alpha.1"
        );
        assert_eq!(
            line(LifecycleEvent::UpdateInstalled { from: "0.8.11".into(), to: "0.8.12-alpha.1".into() }),
            "2026-10-06T13:05:09Z update_installed from=0.8.11 to=0.8.12-alpha.1"
        );
        assert_eq!(
            line(transition("failed")),
            "2026-10-06T13:05:09Z transition from=adding to=failed reason=folder_taken attempt=1/1 domain=NSCocoaErrorDomain code=516 underlying=NSPOSIXErrorDomain:17 message=\"failed\""
        );
    }

    #[test]
    fn the_nserror_message_is_redacted_known_names_and_paths_never_reach_the_file() {
        let message = "could not create /Users/sam/Library/CloudStorage/Beebeeb-Beebeeb/Tax 2025/aangifte.pdf: aangifte.pdf exists";
        let line = format_line(&transition(message), &names(&["Tax 2025/aangifte.pdf"]), at());
        for leaked in ["/Users", "sam", "CloudStorage", "Beebeeb-Beebeeb", "Tax", "aangifte", ".pdf"] {
            assert!(!line.contains(leaked), "{leaked} leaked: {line}");
        }
        assert!(line.contains("[path]"), "{line}");
        assert!(line.contains("[name]"), "{line}");
    }

    #[test]
    fn free_text_cannot_enter_through_versions_or_domains() {
        let line = format_line(
            &LifecycleEvent::UpdateInstalled { from: "0.8.11 \"x\" /Users/sam".into(), to: "1".into() },
            &KnownNames::new(),
            at(),
        );
        assert_eq!(line, "2026-10-06T13:05:09Z update_installed from=0.8.11xUserssam to=1");
    }

    #[test]
    fn rotates_at_one_megabyte_and_keeps_three_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), MAX_BYTES);
        let line = "x".repeat(1023); // 1024 bytes with the newline: 1024 lines fill one file exactly
        for _ in 0..(4 * 1024 + 10) {
            log.append_line(&line).unwrap();
        }
        let mut files: Vec<String> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        files.sort();
        assert_eq!(files, vec!["lifecycle.log", "lifecycle.log.1", "lifecycle.log.2"]);
        for file in &files {
            let len = fs::metadata(dir.path().join(file)).unwrap().len();
            assert!(len <= MAX_BYTES, "{file}: {len}");
        }
        assert_eq!(fs::metadata(dir.path().join("lifecycle.log.1")).unwrap().len(), MAX_BYTES);
        assert_eq!(fs::metadata(dir.path().join("lifecycle.log")).unwrap().len(), 10 * 1024);
    }

    #[test]
    fn tail_reads_the_last_lines_oldest_first_across_rotated_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), 64); // 8 lines of 8 bytes per file
        for i in 0..30 {
            log.append_line(&format!("line {i:02}")).unwrap();
        }
        assert_eq!(log.tail(10), (20..30).map(|i| format!("line {i:02}")).collect::<Vec<_>>());
        assert_eq!(log.tail(1000).first().map(String::as_str), Some("line 08"), "lines 0..8 rotated out");
    }

    /// Uses a local `LifecycleLog`, never the global: other test modules call `event()` in
    /// parallel (Task 9's keys-arrived sites), and a global initialised here would catch them.
    #[test]
    fn the_general_tracing_stream_never_reaches_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), MAX_BYTES);
        let subscriber = tracing_subscriber::fmt().with_writer(std::io::sink).finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(path = "/Users/sam/Documents/secret-plan.pdf", "engine error while syncing secret-plan.pdf");
            write_event(&log, &LifecycleEvent::SignedIn, &KnownNames::new(), at()).unwrap();
            tracing::error!("another engine error mentioning secret-plan.pdf");
        });
        let text = fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.trim_end().ends_with(" signed_in"), "{text}");
        assert!(!text.contains("secret-plan"), "{text}");
        // Nothing wires the file into tracing: no Layer/MakeWriter here, and run()'s tracing
        // init does not mention this module.
        let module = include_str!("lifecycle_log.rs");
        let module = &module[..module.find("#[cfg(test)]\nmod tests").expect("tests module")];
        assert!(!module.contains("Layer") && !module.contains("MakeWriter"), "the lifecycle log must not be a tracing sink");
        let lib = include_str!("lib.rs");
        let init = &lib[lib.find("pub fn run() {").expect("run()")..];
        let init = &init[..init.find(".try_init()").expect("tracing init")];
        assert!(!init.contains("lifecycle"), "{init}");
    }

    /// No test initialises the global, so before `init` it is empty (Windows/Linux behave so).
    #[test]
    fn before_init_events_are_dropped_and_tail_is_empty() {
        event(LifecycleEvent::SignedOut);
        assert!(tail(TAIL_LINES).is_empty());
    }
}
```

In `src-tauri/src/state_db.rs`, next to `queue_diagnostics_with_paths`:

```rust
    /// Every plaintext name this daemon knows locally, for redacting the lifecycle log
    /// (spec 2026-10-06 §8). Same source as the support bundle's redaction.
    pub fn known_names(&self, extra_paths: &[String]) -> Result<KnownNames> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        collect_known_names(&conn, extra_paths)
    }
```

Add a test in state_db's `mod tests`. Put it next to an existing test that uses a temp DB, and copy that test's `StateDb::open` setup and `FileEntry` literal (the same fields as `seeded_db` in `lib.rs:4568`):

```rust
    #[test]
    fn known_names_include_synced_paths_for_the_lifecycle_log() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "f1".into(),
            path: "Tax 2025/aangifte.pdf".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let names = db.known_names(&[]).unwrap();
        let out = crate::diagnostic_redaction::redact_for_export("could not open aangifte.pdf", &names);
        assert!(!out.text.contains("aangifte"), "{}", out.text);
    }
```

If `FileEntry`/`FileStatus`/`ItemKind` are not yet imported in state_db's test module, import them from `super`.

- [ ] **Step 2: Run RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib lifecycle_log > $EVID/t5-red.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t5-red.log
```

Expected: `test result: FAILED. 0 passed; 7 failed` (`not yet implemented`).

- [ ] **Step 3: Implement**

```rust
pub fn format_line(event: &LifecycleEvent, names: &KnownNames, at: SystemTime) -> String {
    let stamp = chrono::DateTime::<chrono::Utc>::from(at).format("%Y-%m-%dT%H:%M:%SZ");
    let body = match event {
        LifecycleEvent::Launch { app_version, macos_version, launch_location } => format!(
            "launch app={} macos={} location={}",
            token(app_version),
            token(macos_version),
            launch_location.as_str()
        ),
        LifecycleEvent::Trigger(trigger) => format!("trigger {}", trigger.as_str()),
        LifecycleEvent::Transition { from, to, reason, error, attempt, max_attempts } => {
            let mut line = format!(
                "transition from={} to={} reason={} attempt={attempt}/{max_attempts}",
                from.as_str(),
                to.as_str(),
                reason.map_or("none", FinderFailureReason::as_str)
            );
            if let Some(error) = error {
                line.push_str(&format!(" domain={} code={}", token(&error.domain), error.code));
                if let Some(underlying) = &error.underlying {
                    line.push_str(&format!(" underlying={}:{}", token(&underlying.domain), underlying.code));
                }
                let redacted = redact_for_export(&error.message, names).text;
                line.push_str(&format!(" message=\"{}\"", redacted.replace(['"', '\n', '\r'], " ")));
            }
            line
        }
        LifecycleEvent::SignedIn => "signed_in".to_string(),
        LifecycleEvent::SignedOut => "signed_out".to_string(),
        LifecycleEvent::UpdateDownloaded { from, to } => format!("update_downloaded from={} to={}", token(from), token(to)),
        LifecycleEvent::UpdateInstalled { from, to } => format!("update_installed from={} to={}", token(from), token(to)),
    };
    format!("{stamp} {body}")
}
```

```rust
    pub fn append_line(&self, line: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let current = fs::metadata(self.path()).map(|m| m.len()).unwrap_or(0);
        if current > 0 && current + line.len() as u64 + 1 > self.max_bytes {
            self.rotate()?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(self.path())?;
        writeln!(file, "{line}")
    }

    fn rotate(&self) -> std::io::Result<()> {
        let oldest = self.rotated(KEEP_FILES - 1);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for n in (1..KEEP_FILES - 1).rev() {
            let from = self.rotated(n);
            if from.exists() {
                fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        fs::rename(self.path(), self.rotated(1))
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let oldest_first = (1..KEEP_FILES).rev().map(|i| self.rotated(i)).chain(std::iter::once(self.path()));
        for path in oldest_first {
            if let Ok(text) = fs::read_to_string(&path) {
                lines.extend(text.lines().map(str::to_string));
            }
        }
        let skip = lines.len().saturating_sub(n);
        lines.split_off(skip)
    }
```

```rust
pub fn event(event: LifecycleEvent) {
    let guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((log, names)) = guard.as_ref() else { return };
    // Known names cost a state.db read; only a transition with an NSError message needs them.
    let known = match &event {
        LifecycleEvent::Transition { error: Some(_), .. } => names(),
        _ => KnownNames::new(),
    };
    if let Err(error) = write_event(log, &event, &known, SystemTime::now()) {
        tracing::warn!(%error, "could not write the lifecycle log");
    }
}

pub fn tail(n: usize) -> Vec<String> {
    GLOBAL.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(log, _)| log.tail(n)).unwrap_or_default()
}
```

- [ ] **Step 4: Run GREEN**

Run the step-2 command into `$EVID/t5-green.log` and expect `test result: ok. 7 passed; 0 failed`. Also run `--lib state_db::tests::known_names_include_synced_paths_for_the_lifecycle_log` and expect `1 passed`.

- [ ] **Step 5: Mutation check (spec §13.1 requires the redaction proof)**

In `format_line`, replace `redact_for_export(&error.message, names).text` with `error.message.clone()`. Expected: `the_nserror_message_is_redacted_…` fails (`/Users leaked`). Restore it. Then make `rotate` keep 4 files (`KEEP_FILES - 1` → `KEEP_FILES`). Expected: `rotates_at_one_megabyte_and_keeps_three_files` fails. Restore it. Then add the comment `// a tracing Layer` above `pub struct LifecycleLog`. Expected: `the_general_tracing_stream_never_reaches_the_file` fails. Remove the comment. Paste all three failures into Notes.

- [ ] **Step 6: Check and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t5-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/lifecycle_log.rs
git commit -m "lifecycle log: typed events, redacted NSError text, 1 MB x 3 rotation (spec A §8)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/lifecycle_log.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
git show --stat HEAD
```

---

## Task 6: Config: `finder_last_failure`, `finder_signed_out_by_choice`, and the 0.8.11 file

**Lane R.** This comes before the driver for the same dependency reason as Task 5: the driver persists both keys. (Round 3: the account identity of R8 is not a config key. It is the owner record inside `state.db`, Task 10.)

**Files:**
- Modify: `src-tauri/src/config.rs` (struct end, after `installed_release_channel` at line ~256; `impl Default` at ~line 322; tests module)
- Create: `src-tauri/tests/fixtures/desktop-0.8.11.toml`

**Interfaces:**
- Consumes: `finder_setup::error::FailureRecord`, `FinderFailureReason`.
- Produces: `DesktopConfig.finder_last_failure: Option<FailureRecord>` and `DesktopConfig.finder_signed_out_by_choice: bool`.

- [ ] **Step 1: Write the fixture and the tests**

`src-tauri/tests/fixtures/desktop-0.8.11.toml`:

```toml
# desktop.toml as desktop 0.8.11 writes it (DesktopConfig at a46146b), carrying the Finder
# failure Guus's Mac saved on 2026-10-06. Values are fixtures; the key set is what matters.
account_id = "11111111-2222-3333-4444-555555555555"
last_signed_in_email = "sam@beebeeb.io"
sync_root = "/Users/sam/Library/Application Support/beebeeb/file-provider-state"
upload_kbps_limit = 0
download_kbps_limit = 0
pause_sync = false
notify_conflicts = true
notify_sync_complete = false
notify_quota_warnings = true
known_folder_backup = []
known_folder_onboarding_seen = false
finder_install_status = "error"
finder_install_last_error = "The file couldn’t be saved because a file with the same name already exists. (NSCocoaErrorDomain 516)"
finder_install_last_attempt_at = 1791291909
finder_install_reason_category = "unknown"
theme = "system"
local_cache_limit_bytes = 0
release_channel = "alpha"
installed_release_channel = "alpha"
```

At the end of `config.rs`'s `mod tests`:

```rust
    // ── Spec 2026-10-06 (macOS Finder setup reconciler) §9 ─────────────────

    #[test]
    fn a_0_8_11_desktop_toml_with_the_old_finder_keys_still_loads() {
        let cfg: DesktopConfig =
            toml::from_str(include_str!("../tests/fixtures/desktop-0.8.11.toml")).expect("a 0.8.11 config parses");
        // The old keys still load: Windows and Linux keep using them; macOS ignores them.
        assert_eq!(cfg.finder_install_status.as_deref(), Some("error"));
        assert_eq!(cfg.finder_install_reason_category.as_deref(), Some("unknown"));
        assert_eq!(cfg.finder_last_failure, None);
        assert!(!cfg.finder_signed_out_by_choice);
        let back = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(!back.contains("finder_last_failure"), "None is not written:\n{back}");
        assert!(!back.contains("finder_signed_out_by_choice"), "false is not written:\n{back}");
    }

    #[test]
    fn the_new_finder_keys_round_trip() {
        use crate::finder_setup::error::FailureRecord;
        use crate::surfaces::phase::FinderFailureReason;
        let mut cfg = DesktopConfig::default();
        cfg.finder_signed_out_by_choice = true;
        cfg.finder_last_failure = Some(FailureRecord {
            reason: FinderFailureReason::FolderTaken,
            domain: "NSCocoaErrorDomain".into(),
            code: 516,
            at: 1_791_291_909,
        });
        let text = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(text.contains("reason = \"folder_taken\""), "{text}");
        let back: DesktopConfig = toml::from_str(&text).expect("parse");
        assert_eq!(back.finder_last_failure, cfg.finder_last_failure);
        assert!(back.finder_signed_out_by_choice);
    }
```

- [ ] **Step 2: Run RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib config::tests > $EVID/t6-red.log 2>&1; echo "rc=$?"; grep -E "error\[E0609\]|no field" $EVID/t6-red.log | head -4
```

Expected: a compile error, `no field 'finder_last_failure' on type 'DesktopConfig'`.

- [ ] **Step 3: Add the fields**

At the end of `pub struct DesktopConfig` (after `installed_release_channel`). The table-valued field comes last so TOML values never follow a table:

```rust
    // ── Spec 2026-10-06 (macOS Finder setup reconciler) §9 ────────────
    //
    // Truth about Finder comes from macOS on every check. These two keys are only memory: the
    // `finder_install_*` keys above stay for Windows/Linux and are no longer written on macOS.
    /// Set when the person signs out (the reconciler then removes Beebeeb from Finder); cleared
    /// at the next sign-in. A startup 401 that discards a revoked session does NOT set it, so
    /// ruling R2 keeps Beebeeb in Finder across that relaunch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finder_signed_out_by_choice: bool,
    /// The last failure, for "Copy details" across a relaunch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finder_last_failure: Option<crate::finder_setup::error::FailureRecord>,
```

And in `impl Default for DesktopConfig`, after `installed_release_channel: None,`:

```rust
            finder_signed_out_by_choice: false,
            finder_last_failure: None,
```

- [ ] **Step 4: Run GREEN**

Run the step-2 command into `$EVID/t6-green.log`. Expected: `test result: ok. N passed; 0 failed`, where N is the old `config::tests` count plus 2. Write N into Notes. Every other `DesktopConfig { … }` literal (config.rs:837; lib.rs:10596, 10749, 10772, 11209, 11224, 11228, 11242, 11360, verified at `c351f98`) already ends in `..DesktopConfig::default()`, so none needs editing. `cargo check --all-targets` in step 6 confirms that.

- [ ] **Step 5: Mutation check**

Remove `skip_serializing_if = "std::ops::Not::not"`. Expected failure: `a_0_8_11_desktop_toml_…` (`false is not written`). Restore it.

- [ ] **Step 6: Check and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t6-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/tests/fixtures/desktop-0.8.11.toml
git commit -m "config: finder_last_failure + finder_signed_out_by_choice; a 0.8.11 desktop.toml still loads (spec A §9)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/config.rs src-tauri/tests/fixtures/desktop-0.8.11.toml
git show --stat HEAD
```

---

## Task 7: The driver: one reconciler task, its handle, its view

**Lane R.** It is generic over `Ports` and `Clock`, so it is fully tested with fakes and an injected clock. It has no macOS code.

`tokio::select!` is available in normal (non-test) macOS builds: `src-tauri/Cargo.toml` lists only `time, rt, rt-multi-thread, sync`, but a normal dependency enables tokio's `full` and with it `macros`. The planner verified this with `cargo tree --offline --locked -p beebeeb-desktop -e features,normal -i tokio --target aarch64-apple-darwin`. The `tokio::time::pause` test utility is NOT enabled, which is why the clock is injected instead.

**Files:**
- Create: `src-tauri/src/finder_setup/driver.rs`
- Modify: `src-tauri/src/finder_setup/mod.rs` (add `pub mod driver;`)

**Interfaces:**
- Consumes: everything from `core` (Task 3), `FailureRecord`/`FpError` (Task 1), `LaunchLocation` (Task 2), `lifecycle_log::{LifecycleEvent, token}` (Task 5).
- Produces (used by Tasks 8, 9 and, as a JSON shape, by Task 14):
  - `pub const FINDER_SETUP_CHANGED_EVENT: &str = "finder-setup-changed"`
  - `pub struct FinderSetupView { setup: FinderSetup, reason: Option<FinderFailureReason>, launch_location: LaunchLocation, attempt: u8, max_attempts: u8, last_failure: Option<FailureRecord> }` (Serialize). JSON: `{"setup":"failed","reason":"folder_taken","launch_location":"applications","attempt":1,"max_attempts":1,"last_failure":{"reason":"folder_taken","domain":"NSCocoaErrorDomain","code":516,"at":1791291909}}`. `FinderSetupView::of(&CoreState, Option<FailureRecord>)` and `FinderSetupView::initial(LaunchLocation)`.
  - `pub enum Event { Trigger(Trigger), Remove { trigger: Trigger, ack: oneshot::Sender<Result<(), FpError>> }, Lock { ack: oneshot::Sender<()> }, AppActivated }`
  - `#[derive(Clone)] pub struct FinderSetupHandle`, with `trigger(&self, Trigger)`, `app_activated(&self)`, `async remove(&self, Trigger, Duration) -> Result<(), String>`, `async lock(&self, Duration) -> Result<(), String>`, `view(&self) -> FinderSetupView`, and under `#[cfg(test)]`: `fixed(FinderSetupView) -> Self`, `for_test(FinderSetupView) -> (Self, mpsc::UnboundedReceiver<Event>)`.
  - `pub trait Ports: Send + 'static { fn run(&mut self, Op) -> impl Future<Output = OpResult> + Send; fn publish(&mut self, &FinderSetupView); fn log(&mut self, LifecycleEvent); fn persist_failure(&mut self, Option<FailureRecord>); fn persist_signed_out_by_choice(&mut self, bool); fn window_visible(&self) -> bool; }`
  - `pub trait Clock: Send + Sync + 'static { fn now(&self) -> Instant; fn unix_now(&self) -> i64; fn sleep_until(&self, Instant) -> impl Future<Output = ()> + Send; }`, and `pub struct SystemClock`.
  - `pub fn start<P: Ports, C: Clock>(ports, clock, launch, persisted: Option<FailureRecord>, policy: RetryPolicy) -> (FinderSetupHandle, impl Future<Output = ()> + Send + 'static)` and `pub fn spawn(...) -> FinderSetupHandle` (the same, on `tauri::async_runtime`).
  - `pub fn copy_details_text(view: &FinderSetupView, app_version: &str, macos_version: &str, lifecycle_tail: &[String]) -> String`
- The spec's `FinderSetupHandle::send(Event)` / `state()` become the typed methods above. `view()` is `state()`.

- [ ] **Step 1: Write `driver.rs` with tests, and `todo!()` bodies in `start`, `copy_details_text` and every `FinderSetupHandle` method**

```rust
//! The single Finder reconciler task (spec §5.4, §9 `finder_setup::driver`). It receives events,
//! runs the one operation the core asks for at a time, and owns the timer. Between two
//! operations it first applies every event that arrived, so a sign-out or a lock always wins
//! over the rest of a check. Generic over `Ports` (OS, engine, app) and `Clock`.

use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot, watch};

use super::core::{self, CoreState, Effect, Input, Next, Op, OpResult, Trigger};
use super::error::{FailureRecord, FpError};
use super::launch_location::LaunchLocation;
use super::policy::RetryPolicy;
use crate::lifecycle_log::{self, LifecycleEvent};
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

pub const FINDER_SETUP_CHANGED_EVENT: &str = "finder-setup-changed";

/// What `finder_setup_state` returns and `finder-setup-changed` carries (spec §5.2, §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FinderSetupView {
    pub setup: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub launch_location: LaunchLocation,
    pub attempt: u8,
    pub max_attempts: u8,
    pub last_failure: Option<FailureRecord>,
}

impl FinderSetupView {
    pub fn of(core: &CoreState, last_failure: Option<FailureRecord>) -> Self {
        Self {
            setup: core.setup,
            reason: core.reason,
            launch_location: core.launch,
            attempt: core.attempt,
            max_attempts: core.max_attempts,
            last_failure,
        }
    }

    pub fn initial(launch: LaunchLocation) -> Self {
        Self::of(&CoreState::new(launch), None)
    }
}

pub enum Event {
    Trigger(Trigger),
    /// Sign-out or Repair: `ack` resolves once the domain removal ran.
    Remove { trigger: Trigger, ack: oneshot::Sender<Result<(), FpError>> },
    /// `ack` resolves once any check is cancelled and the hold is set.
    Lock { ack: oneshot::Sender<()> },
    AppActivated,
}

#[derive(Clone)]
pub struct FinderSetupHandle {
    tx: mpsc::UnboundedSender<Event>,
    view: watch::Receiver<FinderSetupView>,
}

impl FinderSetupHandle {
    pub fn trigger(&self, trigger: Trigger) {
        todo!("Task 7 step 3")
    }

    pub fn app_activated(&self) {
        todo!("Task 7 step 3")
    }

    pub async fn remove(&self, trigger: Trigger, timeout: Duration) -> Result<(), String> {
        todo!("Task 7 step 3")
    }

    pub async fn lock(&self, timeout: Duration) -> Result<(), String> {
        todo!("Task 7 step 3")
    }

    pub fn view(&self) -> FinderSetupView {
        todo!("Task 7 step 3")
    }

    /// A handle whose view never changes and whose events go nowhere.
    #[cfg(test)]
    pub fn fixed(view: FinderSetupView) -> Self {
        Self::for_test(view).0
    }

    /// A handle plus the receiving end of its channel, for tests that answer events by hand.
    #[cfg(test)]
    pub fn for_test(view: FinderSetupView) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (_view_tx, view) = watch::channel(view);
        (Self { tx, view }, rx)
    }
}

pub trait Ports: Send + 'static {
    fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send;
    fn publish(&mut self, view: &FinderSetupView);
    fn log(&mut self, event: LifecycleEvent);
    fn persist_failure(&mut self, record: Option<FailureRecord>);
    fn persist_signed_out_by_choice(&mut self, value: bool);
    fn window_visible(&self) -> bool;
}

/// The schedule's clock (§7: "the schedule and the clock … are injected in tests").
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn unix_now(&self) -> i64;
    fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_now(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default()
    }

    fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send {
        tokio::time::sleep_until(tokio::time::Instant::from_std(at))
    }
}

/// RED stub: an `impl Future` return cannot be satisfied by `todo!()` alone, so the stub returns a
/// real handle and a task that panics when polled. Step 3 replaces both this and the helper.
pub fn start<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    launch: LaunchLocation,
    persisted: Option<FailureRecord>,
    policy: RetryPolicy,
) -> (FinderSetupHandle, impl Future<Output = ()> + Send + 'static) {
    let _ = (ports, clock, persisted, policy);
    let (handle, _rx) = FinderSetupHandle::stub_for_red(FinderSetupView::initial(launch));
    (handle, async { todo!("Task 7 step 3") })
}

impl FinderSetupHandle {
    fn stub_for_red(view: FinderSetupView) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (_view_tx, view) = watch::channel(view);
        (Self { tx, view }, rx)
    }
}

pub fn spawn<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    launch: LaunchLocation,
    persisted: Option<FailureRecord>,
    policy: RetryPolicy,
) -> FinderSetupHandle {
    let (handle, task) = start(ports, clock, launch, persisted, policy);
    tauri::async_runtime::spawn(task);
    handle
}

/// "Copy details" (spec §6.2): app version, macOS version, reason, NSError domain + code, attempt
/// count, the time of the last attempt, and the lifecycle log's tail (§8). Never a path, a file
/// name, the email or the account id: the view carries none, and the tail is already redacted.
pub fn copy_details_text(view: &FinderSetupView, app_version: &str, macos_version: &str, lifecycle_tail: &[String]) -> String {
    todo!("Task 7 step 3")
}
```

Then the test module, at the end of `driver.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::core::{DomainState, Observation, SessionFacts};
    use crate::finder_setup::error::{COCOA_DOMAIN, FILE_PROVIDER_DOMAIN};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeClock {
        t0: Instant,
        now: Arc<Mutex<Instant>>,
    }

    impl FakeClock {
        fn new() -> Self {
            let t0 = Instant::now();
            Self { t0, now: Arc::new(Mutex::new(t0)) }
        }
        fn secs(&self) -> u64 {
            (*self.now.lock().unwrap() - self.t0).as_secs()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
        fn unix_now(&self) -> i64 {
            1_791_291_909 + self.secs() as i64
        }
        fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send {
            {
                let mut now = self.now.lock().unwrap();
                if at > *now {
                    *now = at;
                }
            }
            std::future::ready(())
        }
    }

    #[derive(Default)]
    struct Record {
        ops: Vec<(u64, Op)>,
        published: Vec<FinderSetupView>,
        log: Vec<LifecycleEvent>,
        persisted_failures: Vec<Option<FailureRecord>>,
        signed_out: Vec<bool>,
    }

    struct FakePorts {
        clock: FakeClock,
        record: Arc<Mutex<Record>>,
        domain: DomainState,
        add: Result<(), FpError>,
        /// Runs inside `AddDomain`, before it returns: an event that arrives mid-operation.
        during_add: Option<Box<dyn FnMut() + Send>>,
    }

    impl FakePorts {
        fn new(clock: &FakeClock, domain: DomainState) -> (Self, Arc<Mutex<Record>>) {
            let record = Arc::new(Mutex::new(Record::default()));
            (Self { clock: clock.clone(), record: record.clone(), domain, add: Ok(()), during_add: None }, record)
        }
    }

    impl Ports for FakePorts {
        fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
            self.record.lock().unwrap().ops.push((self.clock.secs(), op));
            let result = match op {
                Op::Observe => OpResult::Observed(Observation {
                    facts: SessionFacts { vault_unlocked: true, auth_present: true, signed_out_by_choice: false },
                    domain: Ok(self.domain),
                }),
                Op::StartEngine => OpResult::EngineStarted(Ok(true)),
                Op::AddDomain => {
                    if let Some(hook) = self.during_add.as_mut() {
                        hook();
                    }
                    OpResult::Added(self.add.clone())
                }
                Op::ReadDomain => OpResult::Domain(Ok(DomainState::Enabled)),
                Op::WaitStable(_) => OpResult::Stable(Ok(())),
                Op::FinishReady => OpResult::Finished(Ok(())),
                Op::StopEngine => OpResult::EngineStopped,
                Op::RemoveDomain => OpResult::Removed(Ok(())),
            };
            std::future::ready(result)
        }
        fn publish(&mut self, view: &FinderSetupView) {
            self.record.lock().unwrap().published.push(view.clone());
        }
        fn log(&mut self, event: LifecycleEvent) {
            self.record.lock().unwrap().log.push(event);
        }
        fn persist_failure(&mut self, record: Option<FailureRecord>) {
            self.record.lock().unwrap().persisted_failures.push(record);
        }
        fn persist_signed_out_by_choice(&mut self, value: bool) {
            self.record.lock().unwrap().signed_out.push(value);
        }
        fn window_visible(&self) -> bool {
            false
        }
    }

    async fn settle(handle: &FinderSetupHandle, done: impl Fn(&FinderSetupView) -> bool) {
        for _ in 0..10_000 {
            if done(&handle.view()) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the driver did not reach the expected view: {:?}", handle.view());
    }

    fn ops(record: &Arc<Mutex<Record>>) -> Vec<Op> {
        record.lock().unwrap().ops.iter().map(|(_, op)| *op).collect()
    }

    #[tokio::test]
    async fn a_launch_runs_one_check_and_publishes_ready() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, None, RetryPolicy::default());
        tokio::spawn(task);
        handle.trigger(Trigger::Launch);
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert_eq!(
            ops(&record),
            vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::ReadDomain, Op::WaitStable(Duration::from_secs(10)), Op::FinishReady]
        );
        let record = record.lock().unwrap();
        assert_eq!(record.published.first().map(|v| v.setup), Some(FinderSetup::Missing), "the first view is published at start");
        assert_eq!(record.published.last().map(|v| v.setup), Some(FinderSetup::Ready));
        assert!(record.log.contains(&LifecycleEvent::Trigger(Trigger::Launch)));
        let to_ready = record.log.iter().filter(|e| matches!(e, LifecycleEvent::Transition { to: FinderSetup::Ready, .. })).count();
        assert_eq!(to_ready, 1, "one log line per transition");
    }

    #[tokio::test]
    async fn sign_out_resolves_its_ack_after_the_removal() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, None, RetryPolicy::default());
        tokio::spawn(task);
        handle.trigger(Trigger::Launch);
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        handle.remove(Trigger::SignOut, Duration::from_secs(5)).await.expect("removed");
        assert_eq!(handle.view().setup, FinderSetup::Missing, "the view is published before the ack");
        assert_eq!(ops(&record).last(), Some(&Op::RemoveDomain));
        assert_eq!(record.lock().unwrap().signed_out, vec![true]);
    }

    #[tokio::test]
    async fn an_event_sent_during_an_operation_waits_for_it_and_wins_over_the_rest_of_the_check() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let slot: Arc<Mutex<Option<FinderSetupHandle>>> = Arc::new(Mutex::new(None));
        let hook_slot = slot.clone();
        ports.during_add = Some(Box::new(move || {
            let handle = hook_slot.lock().unwrap().clone().expect("handle installed");
            handle.trigger(Trigger::SignOut);
        }));
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, None, RetryPolicy::default());
        *slot.lock().unwrap() = Some(handle.clone());
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived);
        for _ in 0..10_000 {
            if ops(&record).contains(&Op::RemoveDomain) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(ops(&record), vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::RemoveDomain]);
        assert_eq!(handle.view().setup, FinderSetup::Missing);
    }

    #[tokio::test]
    async fn transient_failures_follow_the_schedule_on_the_injected_clock() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.add = Err(FpError::new(FILE_PROVIDER_DOMAIN, -2001, "not loaded"));
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, None, RetryPolicy::default());
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived);
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        let starts: Vec<u64> = record.lock().unwrap().ops.iter().filter(|(_, op)| *op == Op::Observe).map(|(t, _)| *t).collect();
        assert_eq!(starts, vec![0, 5, 15, 45]);
        let view = handle.view();
        assert_eq!((view.reason, view.attempt, view.max_attempts), (Some(FinderFailureReason::ExtensionLoading), 4, 4));
        assert_eq!(
            view.last_failure.as_ref().map(|f| (f.domain.as_str(), f.code, f.at)),
            Some((FILE_PROVIDER_DOMAIN, -2001, 1_791_291_909 + 45))
        );
        assert_eq!(record.lock().unwrap().persisted_failures.len(), 1);
    }

    #[tokio::test]
    async fn a_persisted_failure_is_in_the_first_view_and_ready_clears_it() {
        let saved = FailureRecord { reason: FinderFailureReason::FolderTaken, domain: COCOA_DOMAIN.into(), code: 516, at: 1_791_291_909 };
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, Some(saved.clone()), RetryPolicy::default());
        assert_eq!(handle.view().last_failure, Some(saved), "Copy details works across a relaunch, before any check");
        tokio::spawn(task);
        handle.trigger(Trigger::TryAgain);
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert_eq!(handle.view().last_failure, None);
        assert_eq!(record.lock().unwrap().persisted_failures, vec![None]);
    }

    #[tokio::test]
    async fn lock_is_acknowledged_and_holds_until_keys_arrive() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (handle, task) = start(ports, clock.clone(), LaunchLocation::Applications, None, RetryPolicy::default());
        tokio::spawn(task);
        handle.trigger(Trigger::Launch);
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        handle.lock(Duration::from_secs(5)).await.expect("acknowledged");
        let before = ops(&record).len();
        handle.trigger(Trigger::TryAgain);
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        assert_eq!(ops(&record).len(), before, "held: Try again does nothing after a lock");
        handle.trigger(Trigger::KeysArrived);
        for _ in 0..10_000 {
            if ops(&record).len() > before {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(ops(&record).len() > before, "keys arriving lift the hold");
    }

    #[test]
    fn copy_details_names_what_support_needs_and_never_a_path() {
        let view = FinderSetupView {
            setup: FinderSetup::Failed,
            reason: Some(FinderFailureReason::FolderTaken),
            launch_location: LaunchLocation::Applications,
            attempt: 1,
            max_attempts: 1,
            last_failure: Some(FailureRecord { reason: FinderFailureReason::FolderTaken, domain: "NSCocoaErrorDomain".into(), code: 516, at: 1_791_291_909 }),
        };
        let tail = vec!["2026-10-06T13:05:09Z trigger keys_arrived".to_string()];
        let text = copy_details_text(&view, "0.8.12-alpha.1", "26.0", &tail);
        let header = &text[..text.find("Lifecycle log").expect("the log section")];
        assert_eq!(
            header,
            "Beebeeb 0.8.12-alpha.1 on macOS 26.0\nFinder setup: failed\nReason: folder_taken\nError: NSCocoaErrorDomain 516\nLast attempt: 2026-10-06T13:05:09Z\nAttempts: 1 of 1\nLaunched from: applications\n\n"
        );
        assert!(text.ends_with("Lifecycle log (last 1 lines):\n2026-10-06T13:05:09Z trigger keys_arrived\n"), "{text}");
        assert!(!header.contains('/') && !header.contains('@'), "{header}");
    }
}
```

Add `pub mod driver;` to `finder_setup/mod.rs`.

- [ ] **Step 2: Run RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup::driver > $EVID/t7-red.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t7-red.log
```

Expected: `test result: FAILED. 0 passed; 7 failed` (`not yet implemented`). Paste it into Notes.

- [ ] **Step 3: Implement**

Delete `stub_for_red` and the stub `start`, and replace the remaining stubs:

```rust
impl FinderSetupHandle {
    pub fn trigger(&self, trigger: Trigger) {
        let _ = self.tx.send(Event::Trigger(trigger));
    }

    pub fn app_activated(&self) {
        let _ = self.tx.send(Event::AppActivated);
    }

    pub async fn remove(&self, trigger: Trigger, timeout: Duration) -> Result<(), String> {
        let (ack, done) = oneshot::channel();
        self.tx
            .send(Event::Remove { trigger, ack })
            .map_err(|_| "the Finder reconciler is not running".to_string())?;
        match tokio::time::timeout(timeout, done).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(error.to_string()),
            Ok(Err(_)) => Err("the Finder reconciler stopped before removing".to_string()),
            Err(_) => Err(format!("the Finder reconciler did not remove within {}s", timeout.as_secs())),
        }
    }

    pub async fn lock(&self, timeout: Duration) -> Result<(), String> {
        let (ack, done) = oneshot::channel();
        self.tx.send(Event::Lock { ack }).map_err(|_| "the Finder reconciler is not running".to_string())?;
        match tokio::time::timeout(timeout, done).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("the Finder reconciler stopped".to_string()),
            Err(_) => Err(format!("the Finder reconciler did not acknowledge the lock within {}s", timeout.as_secs())),
        }
    }

    pub fn view(&self) -> FinderSetupView {
        self.view.borrow().clone()
    }
}

struct Driver {
    core: CoreState,
    last_failure: Option<FailureRecord>,
    remove_acks: Vec<oneshot::Sender<Result<(), FpError>>>,
    view_tx: watch::Sender<FinderSetupView>,
    policy: RetryPolicy,
}

impl Driver {
    fn publish<P: Ports>(&mut self, ports: &mut P) {
        let view = FinderSetupView::of(&self.core, self.last_failure.clone());
        self.view_tx.send_replace(view.clone());
        ports.publish(&view);
    }

    fn event<P: Ports, C: Clock>(&mut self, event: Event, ports: &mut P, clock: &C) {
        match event {
            Event::Trigger(trigger) => self.input(Input::Trigger(trigger), ports, clock),
            Event::Remove { trigger, ack } => {
                self.remove_acks.push(ack);
                self.input(Input::Trigger(trigger), ports, clock);
            }
            Event::Lock { ack } => {
                self.input(Input::Trigger(Trigger::Lock), ports, clock);
                let _ = ack.send(());
            }
            Event::AppActivated => self.input(Input::AppActivated, ports, clock),
        }
    }

    fn input<P: Ports, C: Clock>(&mut self, input: Input, ports: &mut P, clock: &C) {
        let (core, effects) = core::step(self.core.clone(), input, clock.now(), &self.policy);
        self.core = core;
        for effect in effects {
            match effect {
                Effect::Publish => self.publish(ports),
                Effect::TriggerReceived(trigger) => ports.log(LifecycleEvent::Trigger(trigger)),
                Effect::Transition(t) => ports.log(LifecycleEvent::Transition {
                    from: t.from,
                    to: t.to,
                    reason: t.reason,
                    error: t.error,
                    attempt: t.attempt,
                    max_attempts: t.max_attempts,
                }),
                Effect::PersistFailure(Some((reason, error))) => {
                    let record = FailureRecord { reason, domain: error.domain, code: error.code, at: clock.unix_now() };
                    self.last_failure = Some(record.clone());
                    ports.persist_failure(Some(record));
                }
                Effect::PersistFailure(None) => {
                    if self.last_failure.take().is_some() {
                        ports.persist_failure(None);
                    }
                }
                Effect::PersistSignedOutByChoice(value) => ports.persist_signed_out_by_choice(value),
                Effect::RemoveFinished(result) => {
                    for ack in self.remove_acks.drain(..) {
                        let _ = ack.send(result.clone());
                    }
                }
            }
        }
    }
}

pub fn start<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    launch: LaunchLocation,
    persisted: Option<FailureRecord>,
    policy: RetryPolicy,
) -> (FinderSetupHandle, impl Future<Output = ()> + Send + 'static) {
    let (tx, rx) = mpsc::unbounded_channel();
    let core = CoreState::new(launch);
    let (view_tx, view) = watch::channel(FinderSetupView::of(&core, persisted.clone()));
    let driver = Driver { core, last_failure: persisted, remove_acks: Vec::new(), view_tx, policy };
    (FinderSetupHandle { tx, view }, run(driver, ports, clock, rx))
}

async fn run<P: Ports, C: Clock>(mut driver: Driver, mut ports: P, clock: C, mut rx: mpsc::UnboundedReceiver<Event>) {
    driver.publish(&mut ports);
    loop {
        // Cooperative: an instant fake clock must not starve the runtime.
        tokio::task::yield_now().await;
        // Every event that arrived during the last operation is applied BEFORE the next one is
        // chosen: this is what makes sign-out and lock win over the rest of a check (§5.3).
        while let Ok(event) = rx.try_recv() {
            driver.event(event, &mut ports, &clock);
        }
        match core::next_op(&mut driver.core, clock.now(), &driver.policy) {
            Next::Run(op) => {
                let result = ports.run(op).await;
                driver.input(Input::Done(result), &mut ports, &clock);
            }
            Next::WakeAt(at) => {
                tokio::select! {
                    biased;
                    event = rx.recv() => match event {
                        Some(event) => driver.event(event, &mut ports, &clock),
                        None => return,
                    },
                    () = clock.sleep_until(at) => {
                        let window_visible = ports.window_visible();
                        driver.input(Input::Tick { window_visible }, &mut ports, &clock);
                    }
                }
            }
            Next::Idle => match rx.recv().await {
                Some(event) => driver.event(event, &mut ports, &clock),
                None => return,
            },
        }
    }
}

pub fn copy_details_text(view: &FinderSetupView, app_version: &str, macos_version: &str, lifecycle_tail: &[String]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "Beebeeb {} on macOS {}", lifecycle_log::token(app_version), lifecycle_log::token(macos_version));
    let _ = writeln!(out, "Finder setup: {}", view.setup.as_str());
    let reason = view.reason.or(view.last_failure.as_ref().map(|f| f.reason));
    let _ = writeln!(out, "Reason: {}", reason.map_or("none", FinderFailureReason::as_str));
    match &view.last_failure {
        Some(failure) => {
            let _ = writeln!(out, "Error: {} {}", lifecycle_log::token(&failure.domain), failure.code);
            let at = chrono::DateTime::from_timestamp(failure.at, 0)
                .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let _ = writeln!(out, "Last attempt: {at}");
        }
        None => {
            let _ = writeln!(out, "Error: none");
        }
    }
    let _ = writeln!(out, "Attempts: {} of {}", view.attempt, view.max_attempts.max(view.attempt));
    let _ = writeln!(out, "Launched from: {}", view.launch_location.as_str());
    let _ = writeln!(out, "\nLifecycle log (last {} lines):", lifecycle_tail.len());
    for line in lifecycle_tail {
        let _ = writeln!(out, "{line}");
    }
    out
}
```

`fixed` and `for_test` stay as written in step 1.

- [ ] **Step 4: Run GREEN**

Run the step-2 command into `$EVID/t7-green.log`. Expected: `test result: ok. 7 passed; 0 failed`.

- [ ] **Step 5: Mutation check**

In `run`, move the `while let Ok(event) = rx.try_recv() { … }` loop to *after* the `match`. Expected: `an_event_sent_during_an_operation_…` fails, with `ReadDomain` appearing before `RemoveDomain`. Restore it. Paste the failure into Notes.

- [ ] **Step 6: Check and commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t7-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/finder_setup/driver.rs
git commit -m "finder setup: the single reconciler task, its handle and view (spec A §5.4, §9)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/finder_setup/driver.rs src-tauri/src/finder_setup/mod.rs
git show --stat HEAD
```

---

## Task 8: Wire the reconciler on macOS: ports, commands, launch, popover; retire the old macOS paths

**Lane R.** This is the largest edit to `lib.rs`. The anchors are function names, because line numbers drift. After this task, macOS runs the reconciler at launch, and no macOS code calls the old install path.

**Files:**
- Create: `src-tauri/src/finder_setup/macos_ports.rs`
- Modify: `src-tauri/src/finder_setup/mod.rs` (`#[cfg(target_os = "macos")] pub mod macos_ports;`)
- Modify: `src-tauri/src/lib.rs`: module attributes, `AppState`, new helpers, commands, `setup()`, `popover_snapshot`, the cfg split of `install_finder_location`/`finder_location_state`, the legacy helpers, `reset_macos_integration`'s persist call, and the tests listed below
- Modify: `src-tauri/src/popover_data.rs` (drop `finder_adding`; the reason becomes `FinderFailureReason`)
- Modify: `src-tauri/src/surfaces/phase.rs` (`PopoverSnapshot.finder_reason`)
- Modify: `src-tauri/src/macos_file_provider.rs` + `src-tauri/macos/FileProviderBridge.m` (delete the kept-until-Task-8 items)

**Interfaces:**
- Consumes: Tasks 1–7.
- Produces:
  - `AppState.finder_setup: std::sync::OnceLock<finder_setup::driver::FinderSetupHandle>` (pub). It is set only in `setup()` on macOS.
  - `fn notify_finder(state: &AppState, trigger: finder_setup::core::Trigger)` (Task 9 uses it).
  - Commands, registered on every platform:
    - `finder_setup_state() -> Result<FinderSetupView, String>` (`Err("Finder setup is only available on macOS.")` without a reconciler)
    - `finder_setup_retry() -> Result<(), String>`
    - `finder_setup_copy_details() -> Result<String, String>`
    - `finder_setup_show_app() -> Result<(), String>`
  - `#[cfg(target_os = "macos")] async fn ensure_sync_root_and_engine(app, state: &State<'_, AppState>, root: PathBuf) -> Result<(), String>`
  - `PopoverSnapshot.finder_reason: Option<FinderFailureReason>`; `SnapshotInputs.finder_reason` and `FinderDto.reason` become `Option<FinderFailureReason>`. The JSON shape is unchanged: a snake_case string or null.

- [ ] **Step 1: Write the failing tests**

(a) In `lib.rs`, add a new test module after `popover_wiring_tests`:

```rust
#[cfg(test)]
mod finder_setup_command_tests {
    use super::*;
    use finder_setup::core::Trigger;
    use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
    use finder_setup::launch_location::LaunchLocation;

    #[test]
    fn finder_setup_state_reads_the_reconcilers_view_and_says_so_when_there_is_none() {
        let state = AppState::default();
        assert_eq!(finder_setup_state_impl(&state), Err("Finder setup is only available on macOS.".to_string()));
        let view = FinderSetupView::initial(LaunchLocation::DiskImage);
        let _ = state.finder_setup.set(FinderSetupHandle::fixed(view.clone()));
        assert_eq!(finder_setup_state_impl(&state), Ok(view));
    }

    #[test]
    fn try_again_sends_exactly_one_try_again_trigger() {
        let state = AppState::default();
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        let _ = state.finder_setup.set(handle);
        finder_setup_retry_impl(&state).expect("sent");
        assert!(matches!(rx.try_recv(), Ok(Event::Trigger(Trigger::TryAgain))));
        assert!(rx.try_recv().is_err(), "exactly one");
    }

    #[test]
    fn the_launch_trigger_follows_the_startup_restore() {
        // §5.3 (1): the launch check runs once the restore finished, so it sees the restored keys.
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let restore = source.find("restore_session_on_startup(&h).await;\n").expect("the restore task");
        let launch = source[restore..].find("finder_setup::core::Trigger::Launch").expect("a launch trigger after it");
        assert!(launch < 400, "the launch trigger sits right after the restore, in the same task");
        let driver = source.find("finder_setup::driver::spawn(").expect("setup spawns the reconciler");
        assert!(driver < restore, "the reconciler exists before the restore task can trigger it");
    }
}
```

(b) In `popover_snapshot_command_tests`, add the helper and two tests. The `inject_finder_state` rewrites come in step 3.

```rust
    fn set_finder_view(
        app: &tauri::App<tauri::test::MockRuntime>,
        setup: crate::surfaces::phase::FinderSetup,
        reason: Option<crate::surfaces::phase::FinderFailureReason>,
    ) {
        let view = finder_setup::driver::FinderSetupView {
            setup,
            reason,
            ..finder_setup::driver::FinderSetupView::initial(finder_setup::launch_location::LaunchLocation::Applications)
        };
        let _ = app.state::<AppState>().finder_setup.set(finder_setup::driver::FinderSetupHandle::fixed(view));
    }

    #[test]
    fn without_a_reconciler_macos_reports_missing_and_other_platforms_ready() {
        let api = LoopbackApi::start("200 OK", USAGE);
        with_isolated_env(&api.base, || {
            let app = mock_app_with_account(true, true);
            let snapshot = run_command(&app).expect("the command runs");
            let expected = if cfg!(target_os = "macos") { "missing" } else { "ready" };
            assert_eq!(snapshot["finder"]["setup"], expected);
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_failed_view_reaches_the_snapshot_with_its_reason() {
        let api = LoopbackApi::start("200 OK", USAGE);
        with_isolated_env(&api.base, || {
            let app = mock_app_with_account(true, true);
            set_finder_view(&app, crate::surfaces::phase::FinderSetup::Failed, Some(crate::surfaces::phase::FinderFailureReason::FolderTaken));
            let snapshot = run_command(&app).expect("the command runs");
            assert_eq!(snapshot["phase"], "finder_failed");
            assert_eq!(snapshot["finder"]["setup"], "failed");
            assert_eq!(snapshot["finder"]["reason"], "folder_taken");
            assert_eq!(snapshot["finder"]["reason_line"], "reason: folder_taken");
            assert_eq!(snapshot["storage"]["used_bytes"], 84_300_000_000_i64, "a Finder failure does not hide the storage figure");
        });
    }
```

Run RED:

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup_command_tests > $EVID/t8-red.log 2>&1; echo "rc=$?"; grep -E "error\[|cannot find" $EVID/t8-red.log | head
```

Expected: compile errors: `cannot find function finder_setup_state_impl`, `no field finder_setup on AppState`.

- [ ] **Step 2: `macos_ports.rs`**

```rust
//! The macOS `Ports` (spec §9): the bridge, the engine, the app's event bus, the lifecycle log
//! and `desktop.toml`. The only part of the reconciler that touches the OS; every decision is in
//! `core`.

use std::future::Future;

use tauri::{Emitter, Manager};

use super::core::{Observation, Op, OpResult, SessionFacts};
use super::driver::{FINDER_SETUP_CHANGED_EVENT, FinderSetupView, Ports};
use super::error::{FailureRecord, FpError, app_code};
use crate::AppState;
use crate::config::DesktopConfig;
use crate::lifecycle_log::{self, LifecycleEvent};
use crate::macos_file_provider;

pub struct MacosPorts {
    app: tauri::AppHandle,
}

impl MacosPorts {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

/// A blocking bridge call, off the async executor.
async fn blocking<T: Send + 'static>(call: impl FnOnce() -> Result<T, FpError> + Send + 'static) -> Result<T, FpError> {
    tauri::async_runtime::spawn_blocking(call)
        .await
        .unwrap_or_else(|error| Err(FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, format!("bridge task failed: {error}"))))
}

fn session_facts(app: &tauri::AppHandle) -> SessionFacts {
    let state = app.state::<AppState>();
    let vault_unlocked = state
        .active_account()
        .ok()
        .and_then(|acct| acct.session.lock().ok().map(|guard| guard.is_some()))
        .unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    // Fail safe: an unreadable config never counts as "signed out by choice", so it never removes.
    let signed_out_by_choice = DesktopConfig::load().map(|cfg| cfg.finder_signed_out_by_choice).unwrap_or(false);
    SessionFacts { vault_unlocked, auth_present, signed_out_by_choice }
}

fn update_config(change: impl FnOnce(&mut DesktopConfig) -> bool) {
    match DesktopConfig::load() {
        Ok(mut cfg) => {
            if change(&mut cfg)
                && let Err(error) = cfg.save()
            {
                tracing::warn!(%error, "finder setup: could not save desktop.toml");
            }
        }
        Err(error) => tracing::warn!(%error, "finder setup: could not load desktop.toml"),
    }
}

impl Ports for MacosPorts {
    fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
        let app = self.app.clone();
        async move {
            match op {
                Op::Observe => {
                    let facts = session_facts(&app);
                    let domain = blocking(macos_file_provider::domain_state).await;
                    OpResult::Observed(Observation { facts, domain })
                }
                Op::StartEngine => {
                    let root = crate::config::default_sync_root_suggestion();
                    if let Err(error) = crate::config::ensure_directory(&root) {
                        return OpResult::EngineStarted(Err(FpError::app(app_code::ENGINE_START, error)));
                    }
                    let state = app.state::<AppState>();
                    OpResult::EngineStarted(
                        crate::start_engine_for_pending_finder_install(app.clone(), &state, root)
                            .await
                            .map_err(|error| FpError::app(app_code::ENGINE_START, error)),
                    )
                }
                Op::AddDomain => OpResult::Added(blocking(macos_file_provider::add_domain).await),
                Op::ReadDomain => OpResult::Domain(blocking(macos_file_provider::domain_state).await),
                Op::WaitStable(timeout) => {
                    OpResult::Stable(blocking(move || macos_file_provider::wait_for_domain_ready(timeout)).await)
                }
                Op::FinishReady => {
                    let state = app.state::<AppState>();
                    let root = crate::config::default_sync_root_suggestion();
                    OpResult::Finished(
                        crate::ensure_sync_root_and_engine(app.clone(), &state, root)
                            .await
                            .map_err(|error| FpError::app(app_code::FINISH_READY, error)),
                    )
                }
                Op::StopEngine => {
                    let state = app.state::<AppState>();
                    crate::stop_pending_finder_install_engine(&state, true).await;
                    OpResult::EngineStopped
                }
                Op::RemoveDomain => OpResult::Removed(blocking(macos_file_provider::remove).await),
            }
        }
    }

    fn publish(&mut self, view: &FinderSetupView) {
        let _ = self.app.emit(FINDER_SETUP_CHANGED_EVENT, view);
    }

    fn log(&mut self, event: LifecycleEvent) {
        lifecycle_log::event(event);
    }

    fn persist_failure(&mut self, record: Option<FailureRecord>) {
        update_config(move |cfg| {
            if cfg.finder_last_failure == record {
                return false;
            }
            cfg.finder_last_failure = record;
            true
        });
    }

    fn persist_signed_out_by_choice(&mut self, value: bool) {
        update_config(move |cfg| {
            if cfg.finder_signed_out_by_choice == value {
                return false;
            }
            cfg.finder_signed_out_by_choice = value;
            true
        });
    }

    fn window_visible(&self) -> bool {
        self.app.webview_windows().values().any(|window| window.is_visible().unwrap_or(false))
    }
}
```

Add `#[cfg(target_os = "macos")] pub mod macos_ports;` to `finder_setup/mod.rs`.

- [ ] **Step 3: `lib.rs`**

1. **Module attributes.** Replace `#[allow(dead_code)]` above `mod finder_setup;` and `mod lifecycle_log;` with `#[cfg_attr(not(target_os = "macos"), allow(dead_code))]`, and update the comment: "Only macOS runs the reconciler; on Windows/Linux these types exist for `AppState` and the commands."

2. **`AppState`.** Add the last field, `pub finder_setup: std::sync::OnceLock<finder_setup::driver::FinderSetupHandle>,`, with this doc: "The macOS Finder reconciler (spec 2026-10-06). Set once in `setup()` on macOS; empty on Windows/Linux and in tests that install none." In `impl Default for AppState`, add `finder_setup: std::sync::OnceLock::new(),`.

3. **Helpers.** Add these directly above `fn finder_domain_user_enabled`:

```rust
/// Spec 2026-10-06 §5.3: tell the macOS Finder reconciler something happened. A no-op where no
/// reconciler runs (Windows, Linux, and tests that install none).
fn notify_finder(state: &AppState, trigger: finder_setup::core::Trigger) {
    if let Some(handle) = state.finder_setup.get() {
        handle.trigger(trigger);
    }
}

fn macos_version_string() -> String {
    sysinfo::System::os_version().unwrap_or_else(|| "unknown".to_string())
}

/// The names the lifecycle log scrubs from an NSError message: the same state.db source as the
/// support bundle (task 1685). Empty when there is no state.db yet; the redaction's allow-list
/// still removes every unknown word.
fn lifecycle_known_names() -> diagnostic_redaction::KnownNames {
    let sync_root = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).map(|p| p.to_string_lossy().into_owned());
    match state_db_from_app_local_state_dir() {
        Ok(Some(db)) => db.known_names(&sync_root.into_iter().collect::<Vec<_>>()).unwrap_or_default(),
        _ => diagnostic_redaction::KnownNames::new(),
    }
}

/// Spec §5.5 step 5 (macOS): on `Ready`, save the sync folder and make sure the engine runs.
/// Unlike `persist_sync_root_and_start_engine` it never restarts an engine that is already
/// running: the reconciler started it before `addDomain` (plan "Spec issues" 7), or the
/// relaunch's restore did.
#[cfg(target_os = "macos")]
async fn ensure_sync_root_and_engine(app: tauri::AppHandle, state: &State<'_, AppState>, root: PathBuf) -> Result<(), String> {
    let mut cfg = DesktopConfig::load()?;
    if cfg.sync_root.as_deref() != Some(root.as_path()) {
        cfg.sync_root = Some(root.clone());
        cfg.save()?;
    }
    let acct = state.active_account()?;
    let session = acct
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .as_ref()
        .map(|s| (s.token.clone(), s.master_key));
    let Some((token, key)) = session else {
        return Err("Unlock the vault before adding Beebeeb to Finder.".to_string());
    };
    let mut engine_slot = acct.engine.lock().await;
    if engine_slot.is_none() {
        acct.sync_paused.store(cfg.pause_sync, Ordering::Relaxed);
        *engine_slot = Some(EngineRunner::spawn(app, root, token, key, acct.sync_paused.clone(), acct.auth_health.clone()));
    }
    Ok(())
}

fn finder_setup_state_impl(state: &AppState) -> Result<finder_setup::driver::FinderSetupView, String> {
    state.finder_setup.get().map(|handle| handle.view()).ok_or_else(|| "Finder setup is only available on macOS.".to_string())
}

fn finder_setup_retry_impl(state: &AppState) -> Result<(), String> {
    let handle = state.finder_setup.get().ok_or_else(|| "Finder setup is only available on macOS.".to_string())?;
    handle.trigger(finder_setup::core::Trigger::TryAgain);
    Ok(())
}

/// Spec §9: the reconciler's published state (replaces `finder_location_state` on macOS).
#[tauri::command]
fn finder_setup_state(state: State<'_, AppState>) -> Result<finder_setup::driver::FinderSetupView, String> {
    finder_setup_state_impl(&state)
}

/// "Try again" (§5.3 (4)): a fresh retry budget.
#[tauri::command]
fn finder_setup_retry(state: State<'_, AppState>) -> Result<(), String> {
    finder_setup_retry_impl(&state)
}

/// "Copy details" (§6.2): the text the frontend puts on the pasteboard.
#[tauri::command]
fn finder_setup_copy_details(state: State<'_, AppState>) -> Result<String, String> {
    let view = finder_setup_state_impl(&state)?;
    Ok(finder_setup::driver::copy_details_text(
        &view,
        real_app_version(),
        &macos_version_string(),
        &lifecycle_log::tail(lifecycle_log::TAIL_LINES),
    ))
}

/// "Show in Finder" for `not_in_applications` (plan "Spec issues" 9): reveal the running app so
/// the person can drag it to Applications.
#[tauri::command]
fn finder_setup_show_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = finder_setup::launch_location::current_bundle_path()
            .ok_or_else(|| "This copy of Beebeeb is not an app bundle.".to_string())?;
        app.opener().reveal_item_in_dir(&bundle).map_err(|e| format!("Show in Finder: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Only available on macOS.".to_string())
    }
}
```

Register `finder_setup_state, finder_setup_retry, finder_setup_copy_details, finder_setup_show_app,` in `tauri::generate_handler![…]`, next to `finder_location_state`.

4. **`setup()`.** Directly *before* the `// Resume a fully unlocked session from the OS credential store.` block:

```rust
            // Spec 2026-10-06: the macOS Finder reconciler. One task for the app's life, created
            // before the restore below so the launch trigger always finds it.
            #[cfg(target_os = "macos")]
            {
                if let Some(dir) = lifecycle_log::default_dir() {
                    lifecycle_log::init(dir, lifecycle_known_names);
                }
                let launch = finder_setup::launch_location::current();
                lifecycle_log::event(lifecycle_log::LifecycleEvent::Launch {
                    app_version: real_app_version().to_string(),
                    macos_version: macos_version_string(),
                    launch_location: launch,
                });
                let persisted = DesktopConfig::load().ok().and_then(|cfg| cfg.finder_last_failure);
                let handle = finder_setup::driver::spawn(
                    finder_setup::macos_ports::MacosPorts::new(app.handle().clone()),
                    finder_setup::driver::SystemClock,
                    launch,
                    persisted,
                    finder_setup::policy::RetryPolicy::default(),
                );
                let _ = app.state::<AppState>().finder_setup.set(handle);
            }
```

Then change the restore task from

```rust
                tauri::async_runtime::spawn(async move {
                    restore_session_on_startup(&h).await;
                });
```

to

```rust
                tauri::async_runtime::spawn(async move {
                    restore_session_on_startup(&h).await;
                    // §5.3 (1): the launch check runs once the restore finished, whichever way it
                    // went, so it sees the keys the restore installed.
                    notify_finder(&h.state::<AppState>(), finder_setup::core::Trigger::Launch);
                });
```

5. **`popover_snapshot`.** Replace the whole `// Finder. Only macOS …` block (from `let adding = runtime.finder_adding…` to the closing `};` of `let (finder, finder_reason) = …`) with:

```rust
        // Finder: the reconciler's published view (spec 2026-10-06). It never asks the OS here.
        let (finder, finder_reason) = if cfg!(target_os = "macos") {
            match state.finder_setup.get().map(|handle| handle.view()) {
                Some(view) => (view.setup, view.reason),
                None => (crate::surfaces::phase::FinderSetup::Missing, None),
            }
        } else {
            (crate::surfaces::phase::FinderSetup::Ready, None)
        };
```

In the `use popover_data::{…}` line of that function, drop `finder_setup_for`. Delete `popover_finder_probe_state`, `POPOVER_TEST_FINDER_PROBE`, `FinderProbeGuard` and its `Drop` impl.

6. **The cfg split of the two install-era commands.** Put `#[cfg(not(target_os = "macos"))]` on the existing `#[tauri::command] async fn install_finder_location` and remove its macOS-only lines: the `#[cfg(target_os = "macos")] let root = …`, the `#[cfg(target_os = "macos")] let _ = path;`, and the `let _adding = app.try_state::<popover_data::PopoverRuntime>()…;` statement. The non-macOS body is otherwise byte-for-byte unchanged, which keeps the two `install_finder_location_*` source tests valid. Then, **after** `continue_without_finder_location` (so those tests still slice the non-macOS body first), add:

```rust
/// macOS: Beebeeb adds itself to Finder (spec 2026-10-06, ruling R5). Registered so the command
/// table is the same on every platform; nothing on macOS calls it
/// (tests/finderSetupSourceContract.test.ts).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn install_finder_location(path: Option<String>) -> Result<FinderInstallState, String> {
    let _ = path;
    Err("On macOS Beebeeb adds itself to Finder. Use finder_setup_retry.".to_string())
}
```

Do the same for `finder_location_state`: the existing function becomes `#[cfg(not(target_os = "macos"))]`, and a macOS variant returns `Err("On macOS use finder_setup_state.".to_string())`.

7. **Legacy helpers are deleted on macOS** (spec §6.1). Put `#[cfg(not(target_os = "macos"))]` on: `classify_finder_install_error`, `finder_install_state_from_config`, `finder_state_path` (and delete its `#[cfg(target_os = "macos")]` inner block), `persist_finder_install_result`, `record_finder_install_result`, `finder_install_failure_state`, `clear_finder_install_failure`, `begin_finder_install_attempt`, `finder_install_failed`, `FileProviderStatusOutcome`, `FileProviderInstallOutcome`, `FINDER_USER_DISABLED_MESSAGE`. Delete the macOS variants of `file_provider_installed` and `install_file_provider_domain`; their non-macOS variants already carry `#[cfg(not(target_os = "macos"))]`. In `reset_macos_integration`, put `#[cfg(not(target_os = "macos"))]` on the statement `persist_finder_install_result(&mut cfg, false, None)?;`, and put `#[cfg_attr(target_os = "macos", allow(unused_mut))]` on its `let mut cfg = DesktopConfig::load()?;`.

8. **Tests in `lib.rs` that call the now non-macOS helpers** get `#[cfg(not(target_os = "macos"))]`. At `c351f98` these are `classifies_user_disabled_domain_distinctly_from_the_generic_disabled_case`, `failed_finder_install_is_returned_as_the_saved_state_not_as_an_error`, `a_later_successful_finder_install_clears_the_saved_failure`, `starting_an_attempt_clears_the_saved_failure_but_not_an_installed_state`, `fresh_user_disabled_runtime_result_beats_a_stale_persisted_timeout` and `fresh_genuine_timeout_beats_a_stale_persisted_user_disabled_state`. They keep running on Linux CI as the Windows/Linux proof. If the macOS `cargo check --all-targets` reports another unresolved name in a test, gate that test the same way and list it in Notes.

9. **Popover tests.** In `popover_snapshot_command_tests`, rewrite each `let _probe = inject_finder_state("installed", None);` followed by `let app = mock_app_with_account(…);` as `let app = mock_app_with_account(…); set_finder_view(&app, crate::surfaces::phase::FinderSetup::Ready, None);`. There are three sites, at ~12068, ~12130 and ~12190. Delete `inject_finder_state`, `an_injected_finder_failure_reaches_the_snapshot_not_the_real_domain` (replaced by `a_failed_view_reaches_the_snapshot_with_its_reason`) and `finder_probe_injections_are_scoped_to_their_guards`. That last one tested a global that no longer exists: the view now lives on each app's own `AppState`, so nothing can leak between tests. In `popover_wiring_tests`, delete the block from `// The Finder install marks the popover's f1 state` through its `assert!(clear < adding && adding < slow, …);`, and add:

```rust
        // Spec 2026-10-06: the snapshot reads the reconciler's view and never probes the OS.
        let snapshot = {
            let at = source.find("async fn popover_snapshot(").unwrap();
            &source[at..at + source[at..].find("\n}\n").unwrap()]
        };
        assert!(snapshot.contains("state.finder_setup.get()"));
        assert!(!snapshot.contains("finder_location_state") && !snapshot.contains("spawn_blocking(popover_finder_probe_state)"));
```

10. **`popover_data.rs`.** Delete `finder_adding` from `PopoverRuntime` and from its `Default`, and delete `finder_adding_guard`, `FinderAddingGuard` and its `Drop`. Delete `finder_setup_for` and its tests (`use FinderSetup::*` blocks at ~1071–1105). The reconciler's view replaces it. Change `SnapshotInputs.finder_reason` and `FinderDto.reason` to `Option<crate::surfaces::phase::FinderFailureReason>`. Change `finder_reason_line` to:

```rust
pub fn finder_reason_line(setup: FinderSetup, reason: Option<FinderFailureReason>) -> Option<String> {
    if setup != FinderSetup::Failed {
        return None;
    }
    let line = format!("reason: {}", reason?.as_str());
    Some(if line.chars().count() > 30 {
        let mut cut: String = line.chars().take(29).collect();
        cut.push('…');
        cut
    } else {
        line
    })
}
```

In `assemble`, call it with `finder_reason_line(inputs.finder, inputs.finder_reason)` and set `reason: inputs.finder_reason`. In the `popover_phase(&PopoverSnapshot { … })` literal (~line 539), add `finder_reason: inputs.finder_reason,`. Update the tests: `i.finder_reason = Some("timeout".into())` becomes `Some(FinderFailureReason::Timeout)`, and the `finder_reason_line` test asserts `Some(FinderFailureReason::Timeout)` → `"reason: timeout"`, `None` → `None`, and `FinderSetup::Missing` → `None`. Add a test `every_reason_line_fits_30_characters`, which loops `FinderFailureReason::ALL` and asserts `finder_reason_line(FinderSetup::Failed, Some(r)).unwrap().chars().count() <= 30`.

11. **`surfaces/phase.rs`.** Add `pub finder_reason: Option<FinderFailureReason>,` to `PopoverSnapshot`, with the doc "Why setup failed, alongside `finder` (spec 2026-10-06 §5.2); `PopoverSnapshot` stays `Copy`". Add `finder_reason: None,` in `healthy()`. The `PopoverSnapshot { … }` literals in phase.rs tests all use `..PopoverSnapshot::healthy()` (verified at `c351f98`, lines 186–226), so they need no change.

12. **`macos_file_provider.rs` and the bridge.** Delete `InstallOutcome`, `StatusOutcome`, `InstallDecision`, `decide_install_step`, `status_outcome_from`, `status()`, `install()`, `domain_user_enabled()`, `domain_exists()`, `INSTALL_STABILIZATION_TIMEOUT`, `pub type DomainUserEnabledState`, the FFI declarations of `beebeeb_fp_status`/`beebeeb_fp_domain_exists`, and their tests: `decide_install_step_*` (4) and `status_outcome_*` (5). Their rules now live in core tests: "disabled short-circuits without waiting" is `present_and_turned_off_lands_user_disabled_without_adding`, and "a failed lookup proceeds" is `a_failed_read_after_add_is_not_evidence_and_still_waits`. In `FileProviderBridge.m`, delete `beebeeb_fp_status` and `beebeeb_fp_domain_exists`. In lib.rs, `file_provider_domain_user_enabled` (macOS) becomes:

```rust
#[cfg(target_os = "macos")]
fn file_provider_domain_user_enabled() -> Result<Option<bool>, String> {
    match macos_file_provider::domain_state().map_err(|e| e.to_string())? {
        finder_setup::core::DomainState::Enabled => Ok(Some(true)),
        finder_setup::core::DomainState::Disabled => Ok(Some(false)),
        finder_setup::core::DomainState::NotRegistered => Ok(None),
    }
}
```

- [ ] **Step 4: Run GREEN, both targets**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup_command_tests > $EVID/t8-green-commands.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t8-green-commands.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib popover > $EVID/t8-green-popover.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t8-green-popover.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib macos_file_provider > $EVID/t8-green-bridge.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t8-green-bridge.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t8-check.log 2>&1; echo "rc=$?"
grep -c "^warning" $EVID/t8-check.log
```

Expected: `ok` with 3 passed for the commands module; the popover run passes with its count recorded; the bridge module has 6 + 2 (`test_1698_*`) = 8 passing; `cargo check` gives `rc=0`. Write every count into Notes. Then prove the non-macOS side still compiles. Linux and Windows cannot be cross-tested from the Mac, so this proof is CI's Linux test job on the PR. Note that in the PR description.

- [ ] **Step 5: Mutation check**

In `popover_snapshot`, swap the two branches (`cfg!(target_os = "macos")` → `!cfg!(…)`). Expected: `without_a_reconciler_macos_reports_missing_and_other_platforms_ready` fails on the Mac. Restore it.

- [ ] **Step 6: Commit**

```bash
cd $WT && git add src-tauri/src/finder_setup/macos_ports.rs
git commit -m "finder setup: run the reconciler on macOS; finder_setup_* commands; popover reads its view; retire the macOS install path (spec A §5, §9)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/finder_setup/macos_ports.rs src-tauri/src/finder_setup/mod.rs src-tauri/src/lib.rs src-tauri/src/popover_data.rs src-tauri/src/surfaces/phase.rs src-tauri/src/macos_file_provider.rs src-tauri/macos/FileProviderBridge.m
git show --stat HEAD
```

---

## Task 9: Trigger wiring: keys arrive, sign-out, lock, Repair, app active, updates

**Lane R.** It is the last Rust task. After it, every trigger in spec §5.3 reaches the reconciler.

**Files:**
- Modify: `src-tauri/src/lib.rs`: `apply_session`, `desktop_unlock_with_recovery_phrase`, `unlock_vault`, `clear_session_impl`, `lock_vault`, `reset_macos_integration`, `run()`'s `.on_window_event`, `install_update`, `install_channel_downgrade`, `export_diagnostics`, and new tests

**Interfaces:**
- Consumes: `notify_finder`, `FinderSetupHandle::{remove, lock, app_activated}`, `lifecycle_log::{event, LifecycleEvent}`.
- Produces: `fn keys_arrived(state: &AppState)`, `#[cfg(target_os = "macos")] async fn finder_remove_for(state: &AppState, trigger: Trigger) -> Result<(), String>`, `#[cfg(target_os = "macos")] const FINDER_REMOVE_TIMEOUT: Duration = 30 s`.

- [ ] **Step 1: Write the failing tests**

Add a module to `lib.rs`:

```rust
#[cfg(test)]
mod finder_setup_wiring_tests {
    fn source() -> String {
        include_str!("lib.rs").replace("\r\n", "\n")
    }

    /// The text of a top-level function, from its signature to its closing brace.
    fn body_of(source: &str, signature: &str) -> String {
        let start = source.find(signature).unwrap_or_else(|| panic!("{signature} exists"));
        let rest = &source[start..];
        rest[..rest.find("\n}\n").map(|i| i + 3).unwrap_or(rest.len())].to_string()
    }

    /// The name of the top-level function that contains byte `at`.
    fn enclosing_fn(source: &str, at: usize) -> String {
        let head = &source[..at];
        let start = ["\nfn ", "\nasync fn ", "\npub(crate) async fn ", "\npub fn ", "\npub async fn "]
            .iter()
            .filter_map(|prefix| head.rfind(prefix).map(|i| i + prefix.len()))
            .max()
            .expect("inside a function");
        source[start..].split('(').next().unwrap().trim().to_string()
    }

    /// Spec §5.3 (2), corrected by plan "Spec issues" 3: keys arrive in exactly these functions.
    #[test]
    fn every_function_that_installs_keys_tells_the_finder_reconciler() {
        let source = source();
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").expect("the main test module")];
        let mut installers: Vec<String> = Vec::new();
        for marker in ["*guard = Some(Session {", "*guard = Some(session)"] {
            let mut from = 0;
            while let Some(found) = production[from..].find(marker) {
                installers.push(enclosing_fn(production, from + found));
                from += found + marker.len();
            }
        }
        installers.sort();
        installers.dedup();
        assert_eq!(
            installers,
            vec!["apply_session", "desktop_unlock_with_recovery_phrase", "install_unlocked_session", "restore_session_on_startup"],
            "a new place that puts keys in memory must tell the Finder reconciler (and be added here)"
        );
        // install_unlocked_session's only production caller is unlock_vault.
        assert_eq!(production.matches("install_unlocked_session(&acct, session)").count(), 1);
        for (signature, engine_starts) in [
            ("pub(crate) async fn apply_session(", 1),
            ("async fn desktop_unlock_with_recovery_phrase(", 2),
            ("async fn unlock_vault(", 2),
        ] {
            let body = body_of(production, signature);
            assert_eq!(body.matches("start_engine_if_possible(").count(), engine_starts, "{signature}");
            assert_eq!(body.matches("keys_arrived(").count(), engine_starts, "{signature}: every keys-arrive branch tells the reconciler");
        }
        // The startup restore is covered by the launch trigger (Task 8's test pins its order).
        // The two login commands store only a token: they never install keys.
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let body = body_of(production, signature);
            assert!(!body.contains("Some(Session {") && !body.contains("master_key"), "{signature} installs keys now: wire keys_arrived");
        }
    }

    #[test]
    fn sign_out_lock_and_repair_reach_the_reconciler_before_the_engine_stops() {
        let source = source();
        let clear = body_of(&source, "async fn clear_session_impl(");
        let remove = clear.find("finder_remove_for(state, finder_setup::core::Trigger::SignOut)").expect("sign-out tells the reconciler");
        let engine = clear.find("acct.engine.lock().await").expect("sign-out stops the engine");
        assert!(remove < engine, "the reconciler is told before the engine stops");
        assert!(!clear.contains("remove_file_provider_domain()"), "no second, direct removal");
        let lock = body_of(&source, "async fn lock_vault(");
        assert!(lock.find("handle.lock(").expect("lock tells the reconciler") < lock.find("acct.engine.lock().await").unwrap());
        let repair = body_of(&source, "async fn reset_macos_integration(");
        assert!(repair.contains("finder_remove_for(&state, finder_setup::core::Trigger::Repair)"));
    }

    #[test]
    fn becoming_active_and_updates_reach_the_reconciler_and_the_log() {
        let source = source();
        // Only production code: this test's own string literals would otherwise match themselves.
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").expect("the main test module")];
        let run = body_of(production, "pub fn run() {");
        assert!(run.contains("tauri::WindowEvent::Focused(true) = event") && run.contains("handle.app_activated()"));
        for signature in ["async fn install_update(", "async fn install_channel_downgrade("] {
            let body = body_of(production, signature);
            assert!(body.contains("LifecycleEvent::UpdateDownloaded"), "{signature}");
            assert!(body.contains("LifecycleEvent::UpdateInstalled"), "{signature}");
        }
    }

    /// Spec §8: "Copy details" (Task 8) and the 1685 support bundle include the log's last 200 lines.
    #[test]
    fn the_support_bundle_carries_the_lifecycle_tail() {
        let bundle = super::with_lifecycle_tail(
            serde_json::json!({ "diagnostics_format": 2 }),
            vec!["2026-10-06T13:05:09Z signed_in".to_string()],
        );
        assert_eq!(bundle["lifecycle_log"], serde_json::json!(["2026-10-06T13:05:09Z signed_in"]));
        assert_eq!(bundle["diagnostics_format"], 2, "additive key: the format number is unchanged");
        let source = source();
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").expect("the main test module")];
        assert!(body_of(production, "fn export_diagnostics()").contains("with_lifecycle_tail("));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sign_out_waits_for_the_reconciler_removal() {
        use super::*;
        use finder_setup::core::Trigger;
        use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("finder-signout".into()));
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(finder_setup::launch_location::LaunchLocation::Applications));
        let _ = state.finder_setup.set(handle);
        let removed = Arc::new(AtomicBool::new(false));
        let flag = removed.clone();
        let responder = tokio::spawn(async move {
            if let Some(Event::Remove { trigger: Trigger::SignOut, ack }) = rx.recv().await {
                flag.store(true, SeqCst);
                let _ = ack.send(Ok(()));
            }
        });
        let _ = clear_session_impl(&state).await;
        assert!(removed.load(SeqCst), "sign-out waited for the reconciler's removal");
        responder.await.unwrap();
    }
}
```

Run RED:

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup_wiring_tests > $EVID/t9-red.log 2>&1; echo "rc=$?"; grep -E "test result:|panicked at" $EVID/t9-red.log | head
```

Expected, in two rounds. First `rc=101` with a compile error, `cannot find function with_lifecycle_tail in module super`: that is the RED of the support-bundle test. Add item 9 of step 2 (the function only) and rerun. Now the module compiles, and the other source tests fail on their assertions (`keys_arrived` count 0, `finder_remove_for` missing, `Focused(true)` missing), as does the macOS behavioural test. Paste both outputs into Notes.

- [ ] **Step 2: Implement**

1. Helpers next to `notify_finder`:

```rust
/// Keys arrived on this Mac (§5.3 (2)). The functions that put a `Session` in memory call this
/// right after the engine start, whatever its outcome. `finder_setup_wiring_tests` keeps that list
/// complete.
fn keys_arrived(state: &AppState) {
    lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedIn);
    notify_finder(state, finder_setup::core::Trigger::KeysArrived);
}

#[cfg(target_os = "macos")]
const FINDER_REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Sign-out (§5.3 (5)) and Repair (§5.3 (6)) on macOS: the reconciler cancels any check and
/// removes the domain; this waits for that removal (bounded), so the caller's next step cannot
/// race a check. Without a reconciler (never on a shipped Mac) it removes directly, as before.
#[cfg(target_os = "macos")]
async fn finder_remove_for(state: &AppState, trigger: finder_setup::core::Trigger) -> Result<(), String> {
    match state.finder_setup.get() {
        Some(handle) => handle.remove(trigger, FINDER_REMOVE_TIMEOUT).await,
        None => remove_file_provider_domain(),
    }
}
```

2. **`apply_session`**. Replace its last lines

```rust
    start_engine_if_possible(
        app,
        state,
        token_clone,
        *master_key,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await?;
    Ok(())
```

with

```rust
    let started = start_engine_if_possible(
        app,
        state,
        token_clone,
        *master_key,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await;
    keys_arrived(state);
    started
```

3. **`desktop_unlock_with_recovery_phrase`** and **`unlock_vault`**. Each has two `start_engine_if_possible(…).await?;` sites: the early-return `if let Some((token, master_key)) = existing` branch, and the end of the `work` block. Apply the same shape to each:

```rust
            let started = start_engine_if_possible(
                app,
                &state,
                token,
                master_key,
                #[cfg(target_os = "windows")]
                &_transition,
            )
            .await;
            keys_arrived(&state);
            return started;
```

For the early branch, use `return started;`. At the end of the block, use a plain `started` as the block's tail expression in place of `.await?; Ok(())`.

4. **`clear_session_impl`**. Directly after `let acct = state.active_account()?;`:

```rust
    // Spec 2026-10-06 §5.3 (5): a sign-out by choice removes Beebeeb from Finder. The reconciler
    // cancels any check first, so no check can start an engine after the stop below (plan "Spec
    // issues" 8). Best-effort and bounded, like the removal it replaces.
    #[cfg(target_os = "macos")]
    if let Err(error) = finder_remove_for(state, finder_setup::core::Trigger::SignOut).await {
        tracing::warn!(%error, "Finder removal on sign-out failed (best-effort)");
    }
```

Delete the old block that begins `// macOS: remove the Finder File Provider domain on sign-out` and ends with its `#[cfg(target_os = "macos")] { if let Err(error) = remove_file_provider_domain() { … } }`. Directly before the final `Ok(SignOutOutcome::Completed)`, add `lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedOut);`.

5. **`lock_vault`**. Directly after `let acct = state.active_account()?;`:

```rust
    // Spec A (plan "Spec issues" 8): cancel any Finder check and hold BEFORE the engine stops, so
    // the reconciler cannot start an engine with keys this lock is about to clear.
    #[cfg(target_os = "macos")]
    if let Some(handle) = state.finder_setup.get()
        && let Err(error) = handle.lock(FINDER_REMOVE_TIMEOUT).await
    {
        tracing::warn!(%error, "the Finder reconciler did not acknowledge the lock");
    }
```

6. **`reset_macos_integration`**. Replace `let removed_file_provider_domain = match remove_file_provider_domain() {` with:

```rust
    // Spec §5.3 (6): Repair removes; the reconciler then runs one fresh check by itself, so a
    // signed-in Mac ends with Beebeeb back in Finder.
    #[cfg(target_os = "macos")]
    let removal = finder_remove_for(&state, finder_setup::core::Trigger::Repair).await;
    #[cfg(not(target_os = "macos"))]
    let removal = remove_file_provider_domain();
    let removed_file_provider_domain = match removal {
```

7. **App becomes active**. Inside `.on_window_event(|window, event| { … })` in `run()`, before the `CloseRequested` handling:

```rust
            // Spec §7: one read-only userEnabled check when the app becomes active.
            if let tauri::WindowEvent::Focused(true) = event
                && let Some(handle) = window.state::<AppState>().finder_setup.get()
            {
                handle.app_activated();
            }
```

8. **Updates** (§8 "update downloaded or installed: from and to versions"). In `install_update`, inside `Some(u) => {`, before `u.download_and_install(`:

```rust
            let from_version = real_app_version().to_string();
            let to_version = u.version.to_string();
            let versions = (from_version.clone(), to_version.clone());
```

Change the second closure to `move || { tracing::info!("update installed — relaunching"); lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded { from: versions.0, to: versions.1 }); }`. After `.map_err(|e| e.to_string())?;`, add `lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled { from: from_version, to: to_version });`. Make the same change in `install_channel_downgrade`.

9. **Support bundle** (§8: "'Copy details' and the 1685 support bundle include its last 200 lines"). Add next to `export_diagnostics_from`:

```rust
/// Spec 2026-10-06 §8: the support bundle carries the lifecycle log's last 200 lines (already
/// redacted when written). An additive key: `diagnostics_format` is unchanged.
fn with_lifecycle_tail(mut bundle: serde_json::Value, tail: Vec<String>) -> serde_json::Value {
    if let Some(map) = bundle.as_object_mut() {
        map.insert("lifecycle_log".to_string(), serde_json::json!(tail));
    }
    bundle
}
```

`export_diagnostics()` then becomes:

```rust
fn export_diagnostics() -> Result<serde_json::Value, String> {
    let cfg = DesktopConfig::load()?;
    let bundle = match cfg.sync_root {
        None => export_diagnostics_from(None, Path::new(""))?,
        Some(root) => {
            let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
            export_diagnostics_from(Some(&root), &db_path)?
        }
    };
    Ok(with_lifecycle_tail(bundle, lifecycle_log::tail(lifecycle_log::TAIL_LINES)))
}
```

Keep its existing attributes (`#[tauri::command]` and any others) unchanged.

- [ ] **Step 3: Run GREEN, then the whole crate**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup_wiring_tests > $EVID/t9-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t9-green.log
$LOCK cargo-build -- cargo test --locked > $EVID/t9-cargo-test.log 2>&1; echo "rc=$?" > $EVID/t9-cargo-test.exit; cat $EVID/t9-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py $EVID/t9-cargo-test.log --cargo-exit-code $(cut -d= -f2 $EVID/t9-cargo-test.exit)
grep "test result:" $EVID/t9-cargo-test.log
```

Expected: the wiring module passes 5 tests on the Mac (4 elsewhere), and every binary prints `test result: ok. N passed; 0 failed` with N > 0 except the empty `main` harness. The count guard script exits 0. Paste every `test result:` line into Notes, with the lib count next to the Task 1 baseline.

- [ ] **Step 4: Mutation check**

Delete the `keys_arrived(&state);` line from `unlock_vault`'s early branch. Expected: `every_function_that_installs_keys_tells_the_finder_reconciler` fails with `async fn unlock_vault(: every keys-arrive branch tells the reconciler` (1 ≠ 2). Restore it. Then move the `finder_remove_for(state, …SignOut)` block below the engine stop. Expected: `sign_out_lock_and_repair_…` fails. Restore it.

- [ ] **Step 5: Commit**

```bash
cd $WT && git commit -m "finder setup: every keys-arrive path, sign-out, lock, Repair, app active and updates reach the reconciler and the log (spec A §5.3, §8)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/lib.rs
git show --stat HEAD
```

- [ ] **Step 6: Open Lane R's PR** (lead)

Round 2: this step runs after Task 12 and after the security review (Task 11 step 9), not directly after Task 9. The lead runs the Task 21 step 2 Rust gate in a fresh tree at the branch head, pushes, and opens the PR. The PR description lists Spec issues 1–23 (Spec issue 17 in its neutral wording only) and quotes the security review's findings with how each was handled, the evidence directory, and the fact that the macOS-only tests (bridge, `a_failed_view…`, `sign_out_waits…`) ran on the Mac gate.

---

## Task 10: Local data is bound to the account that created it (R10, backend, all platforms)

**Lane R.** Ruling R10 (Guus, 2026-10-06, "Fold into 1834"): spec A also owns "local data is bound to the account that created it". This task is the fix boundary of the private workspace task. Read that file before starting: it holds the background and the device check, which stay out of this public repo. Write about this task in neutral hardening terms only, in commits, comments and the PR ("local data is bound to the account that created it; a different account never reuses it"). It comes before the R8 tasks because R8's account check (Task 11) reads the owner this task records. Shared code covers macOS, Linux and Windows. **Security-sensitive:** the mandatory review in Task 11 step 9 covers this task too.

**One primitive, and where it lives (plan decision, with evidence).** The owner of the local data is stored **inside `state.db`**, in the existing `sync_state` key-value table (`state_db.rs:800-803`), as `owner_user_id` and `owner_email`. Round 2's `desktop.toml` key `signed_in_user_id` is dropped (Task 6 no longer adds it). There is now one record, and it is this one, for three reasons:

1. It sits in the same SQLite file as the data it protects. A reset and the new owner can never disagree across two files.
2. The sign-out purge already refuses to depend on `desktop.toml` health (`lib.rs:2297-2311`, Task 1538 Codex P1). A missing owner fails closed (reset). An owner kept in a config file that can be missing or reset would turn a config problem into discarded unsent changes for the same account.
3. It goes when the data goes: a sign-out purge clears it.

The session's own identity is not stored anywhere new. It is the profile its sign-in fetched (`acct.cached_profile`, set at `lib.rs:1105` and `1227`; the startup probe adds it in Task 12) when that profile matches the session's email, and otherwise the session email alone (`acct.auth_email`). So an offline relaunch compares by email.

**Where the check sits (plan decision, with evidence).** The private task's fix boundary names `start_engine_if_possible`. Engines start in five places, though: `start_engine_if_possible` (`lib.rs:851`), `persist_sync_root_and_start_engine` (`lib.rs:2562`), `start_engine_for_pending_finder_install` (`lib.rs:2612`), `pick_sync_root` (`lib.rs:4909`), and Task 8's `ensure_sync_root_and_engine`. Every sign-in path ends in the first one. Onboarding's folder pick and the Finder reconciler start engines without it. So the check sits at the spawn itself: `spawn_bound_engine` binds first and is the only `EngineRunner::spawn(` left in production code. A source test pins that.

**The decision** (pure, `account_binding::decide`):

| Recorded owner | This session compared with the owner | Local data | Result |
|---|---|---|---|
| none | — | none | start, and record this session as the owner |
| none | — | some | reset, then start (fail closed) |
| some | same account (user id when both know it, else email, ignoring case) | any | start, keep everything |
| some | different account | any | reset, then start |
| some | cannot be compared (no field in common) | any | refuse: start nothing, delete nothing |

- **Reset** = the sign-out purge (`purge_local_state_files`: the queue, staged payloads and cache files) plus every remaining row of the previous account, then the new owner. A reset that fails returns an error, and no engine starts. On macOS a reset then asks the reconciler for a Repair, without waiting for it, so Finder drops the old listing and adds Beebeeb back for the new account. The gate never waits on the reconciler: the reconciler itself starts engines through this gate.
- **Windows refuses where macOS and Linux reset** (`OTHER_ACCOUNT_ON_WINDOWS`). Windows sign-out never discards unsent changes silently (`state_db.rs:2766`, `windows_cf/signout.rs:24`). The tested way to clear a Windows PC is that sign-out, which also unregisters the Cloud Files root. The resulting Windows gap is Spec issue 17, ruled R11: refuse here, and the escape hatch is private task 1837.
- **Upgrade path.** Installs from before this task have local data but no owner. `restore_session_on_startup` first adopts the account this computer's Keychain still names (its email) as the owner, before the probe and before anything starts. R8's same-account re-sign-in (Task 11) records the owner too. Any other unrecorded data is reset.
- **Sign-out by choice.** `purge_all_local_state` also deletes the staged-payload rows (and returns their files for deletion) and forgets the owner. On macOS and Linux a failed purge now stops the sign-out (`SIGN_OUT_PURGE_FAILED`), as Windows already does (`lib.rs:1751`). Today it logs a warning and returns `Completed` (`lib.rs:1829-1843`). The engine is already stopped at that point, so trying again is safe.

**Files:**
- Create: `src-tauri/src/account_binding.rs` (pure decision, an executor over a `LocalData` trait, unit tests with a fake)
- Modify: `src-tauri/src/state_db.rs`:
  - `ACCOUNT_TABLES`, `DEVICE_TABLES`, `owner`, `set_owner`, `has_account_data`, `clear_account_data`
  - `purge_all_local_state` also clears `staged_payloads` and the owner
  - tests
- Modify: `src-tauri/src/lib.rs`:
  - `mod account_binding;`, `StateDbLocalData`, `session_identity`, `bind_local_data_to_session`, `spawn_bound_engine` and its five callers
  - `adopt_unbound_local_data_at_startup` (called from `restore_session_on_startup`)
  - the stopping sign-out purge, `SIGN_OUT_PURGE_FAILED`
  - `mod account_binding_tests`

**Interfaces:**
- Consumes: Task 8's `ensure_sync_root_and_engine` and `notify_finder`, and `finder_setup::core::Trigger::Repair` (Task 3).
- Produces:
  - `account_binding::{Identity { user_id: Option<String>, email: Option<String> }, Identity::new(Option<&str>, Option<&str>), Identity::is_known, same_account(&Identity, &Identity) -> Option<bool>, Binding { Proceed, Reset, Refuse }, decide(Option<&Identity>, &Identity, bool) -> Binding, trait LocalData, Bound { Kept, Reset }, bind_before_engine_start(&dyn LocalData, &Identity, bool) -> Result<Bound, String>, adopt_unbound(&dyn LocalData, &Identity) -> Result<bool, String>, IDENTITY_UNKNOWN, OTHER_ACCOUNT_ON_WINDOWS}`.
  - `StateDb::{owner() -> Result<Option<Identity>>, set_owner(&Identity) -> Result<()>, has_account_data() -> Result<bool>, clear_account_data() -> Result<()>}`.
  - lib.rs: `struct StateDbLocalData { db: StateDb, sync_root: Option<PathBuf> }`, `fn session_identity(&AccountRuntime) -> Identity`, `fn spawn_bound_engine(…) -> Result<EngineRunner, String>`. Task 11 adds `local_data_owner` and `record_local_data_owner` on top of `StateDb::owner`/`set_owner`.

- [ ] **Step 1: The census test alone, RED on today's code**

Add a new test module at the very end of `lib.rs` (after `mod tests`, so the `production` slice below excludes it):

```rust
/// R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it.
#[cfg(test)]
mod account_binding_tests {
    use super::*;

    fn production(source: &str) -> &str {
        &source[..source.find("#[cfg(test)]\nmod tests {").expect("the main test module")]
    }

    fn body_of<'a>(production: &'a str, signature: &str) -> &'a str {
        let body = &production[production.find(signature).unwrap_or_else(|| panic!("{signature}"))..];
        &body[..body.find("\n}\n").expect("function end")]
    }

    /// Every engine start binds the local data to the session's account first, and a failed
    /// binding returns before any engine exists.
    #[test]
    fn every_engine_start_is_bound_first() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = production(&source);
        assert_eq!(
            production.matches("EngineRunner::spawn(").count(),
            1,
            "every engine start goes through spawn_bound_engine"
        );
        let gate = body_of(production, "fn spawn_bound_engine(");
        let bind = gate.find("bind_local_data_to_session(").expect("it binds");
        let spawn = gate.find("EngineRunner::spawn(").expect("then spawns");
        assert!(bind < spawn, "the binding runs before the engine exists");
        assert!(gate[bind..spawn].contains(")?;"), "a failed binding returns before the spawn");
    }
}
```

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib account_binding_tests > $EVID/t10-red-census.log 2>&1; echo "rc=$?"; grep -E "panicked|left|right|test result:" $EVID/t10-red-census.log | head
```

Expected: `rc=101`, `every engine start goes through spawn_bound_engine`, `left: 5` (4 at `c351f98` plus Task 8's `ensure_sync_root_and_engine`; 4 if Task 8 removed one), `right: 1`. This is the RED on today's code that the private task asks for. Paste it into Notes and into the private task's Notes.

- [ ] **Step 2: The pure module, tests first**

`src-tauri/src/account_binding.rs`:

```rust
//! Ruling R10 (spec 2026-10-06 §5.6): the local data on this computer belongs to the account that
//! created it, and a different account never reuses it. "Local data" is everything in `state.db`
//! (the queue, staged uploads, upload sessions, file rows, Finder anchors, activity) and the files it
//! points at (staged payloads, the decrypted cache). Pure: the caller gathers the facts and this
//! decides. `lib.rs` runs it before every engine start (`spawn_bound_engine`).

/// Who an account is, as far as this computer knows: the server user id when known, else the
/// account email (an offline relaunch, or local data recorded before the id was known).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub user_id: Option<String>,
    pub email: Option<String>,
}

impl Identity {
    /// Trims both fields; an empty value is unknown.
    pub fn new(user_id: Option<&str>, email: Option<&str>) -> Self {
        let clean = |value: Option<&str>| value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        Self { user_id: clean(user_id), email: clean(email) }
    }

    pub fn is_known(&self) -> bool {
        self.user_id.is_some() || self.email.is_some()
    }
}

/// `Some(true)`: the same account. `Some(false)`: a different one. `None`: nothing to compare.
/// The user id decides whenever both sides know it; otherwise the email decides, ignoring case.
pub fn same_account(a: &Identity, b: &Identity) -> Option<bool> {
    todo!("step 3")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// This session's account owns the local data, or there is none: start, keep everything.
    Proceed,
    /// The local data belongs to another account, or to no recorded account: reset it first.
    Reset,
    /// An owner is recorded but cannot be compared with this session: start nothing, delete nothing.
    Refuse,
}

pub fn decide(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> Binding {
    todo!("step 3")
}

/// The local data, as the binding needs it. `lib.rs` `StateDbLocalData` is the real one.
pub trait LocalData {
    fn owner(&self) -> Result<Option<Identity>, String>;
    fn has_account_data(&self) -> Result<bool, String>;
    /// The sign-out purge plus every remaining row. An error means nothing may start.
    fn reset(&self) -> Result<(), String>;
    fn record_owner(&self, owner: &Identity) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Kept,
    Reset,
}

pub const IDENTITY_UNKNOWN: &str = "Beebeeb couldn’t confirm which account this computer’s local files belong to, so sync didn’t start. Connect to the internet and open Beebeeb again.";
pub const OTHER_ACCOUNT_ON_WINDOWS: &str = "This PC still holds local files of another Beebeeb account, so sync didn’t start. Sign out, then sign in again.";

/// Run before every engine start. `reset_allowed` is false on Windows, where an account change goes
/// through the Windows sign-out instead. An error means: do not start the engine.
pub fn bind_before_engine_start(data: &dyn LocalData, session: &Identity, reset_allowed: bool) -> Result<Bound, String> {
    todo!("step 3")
}

/// The upgrade path, run once at startup before anything starts: local data from before R10 has no
/// owner, and the account this computer's Keychain still names has been using it. Never replaces a
/// recorded owner and never invents one.
pub fn adopt_unbound(data: &dyn LocalData, keychain_account: &Identity) -> Result<bool, String> {
    todo!("step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn id(user_id: Option<&str>, email: Option<&str>) -> Identity {
        Identity::new(user_id, email)
    }

    #[derive(Default)]
    struct Fake {
        owner: RefCell<Option<Identity>>,
        data: RefCell<bool>,
        reset_fails: bool,
        resets: RefCell<u32>,
        records: RefCell<Vec<Identity>>,
    }

    impl Fake {
        fn holding(owner: Option<Identity>, data: bool) -> Self {
            let fake = Fake::default();
            *fake.owner.borrow_mut() = owner;
            *fake.data.borrow_mut() = data;
            fake
        }
    }

    impl LocalData for Fake {
        fn owner(&self) -> Result<Option<Identity>, String> {
            Ok(self.owner.borrow().clone())
        }
        fn has_account_data(&self) -> Result<bool, String> {
            Ok(*self.data.borrow())
        }
        fn reset(&self) -> Result<(), String> {
            if self.reset_fails {
                return Err("disk I/O error".into());
            }
            *self.resets.borrow_mut() += 1;
            *self.data.borrow_mut() = false;
            *self.owner.borrow_mut() = None;
            Ok(())
        }
        fn record_owner(&self, owner: &Identity) -> Result<(), String> {
            self.records.borrow_mut().push(owner.clone());
            *self.owner.borrow_mut() = Some(owner.clone());
            Ok(())
        }
    }

    #[test]
    fn the_r10_table() {
        let a = id(Some("u-a"), Some("a@beebeeb.io"));
        let a_offline = id(None, Some("A@Beebeeb.io"));
        let b = id(Some("u-b"), Some("b@beebeeb.io"));
        let only_an_id = id(Some("u-a"), None);
        let nobody = Identity::default();
        for (name, owner, session, has_local_data, expected) in [
            ("a fresh computer", None, &b, false, Binding::Proceed),
            ("unrecorded local data (fail closed)", None, &a, true, Binding::Reset),
            ("the same account, by id", Some(&a), &a, true, Binding::Proceed),
            ("the same account offline, by email in any case", Some(&a), &a_offline, true, Binding::Proceed),
            ("another account", Some(&a), &b, true, Binding::Reset),
            ("another account, nothing left locally", Some(&a), &b, false, Binding::Reset),
            ("nothing in common to compare", Some(&only_an_id), &a_offline, true, Binding::Refuse),
            ("a session nobody can identify", Some(&a), &nobody, true, Binding::Refuse),
        ] {
            assert_eq!(decide(owner, session, has_local_data), expected, "{name}");
        }
    }

    #[test]
    fn the_user_id_decides_before_the_email() {
        assert_eq!(same_account(&id(Some("u-a"), Some("x@beebeeb.io")), &id(Some("u-b"), Some("x@beebeeb.io"))), Some(false));
        assert_eq!(same_account(&id(Some("u-a"), Some("old@beebeeb.io")), &id(Some("u-a"), Some("new@beebeeb.io"))), Some(true));
        assert_eq!(same_account(&id(None, Some("  ")), &id(None, Some(""))), None, "an empty email never matches");
    }

    #[test]
    fn a_failed_reset_blocks_the_engine_and_records_nothing() {
        let fake = Fake { reset_fails: true, ..Fake::holding(Some(id(Some("u-a"), Some("a@beebeeb.io"))), true) };
        assert_eq!(
            bind_before_engine_start(&fake, &id(Some("u-b"), Some("b@beebeeb.io")), true),
            Err("disk I/O error".to_string())
        );
        assert!(fake.records.borrow().is_empty(), "the new account is never recorded over data still there");
    }

    #[test]
    fn windows_refuses_instead_of_resetting() {
        let fake = Fake::holding(Some(id(Some("u-a"), Some("a@beebeeb.io"))), true);
        assert_eq!(
            bind_before_engine_start(&fake, &id(Some("u-b"), None), false),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
        assert_eq!(*fake.resets.borrow(), 0);
        assert!(fake.records.borrow().is_empty());
    }

    #[test]
    fn refusing_deletes_nothing_and_records_nothing() {
        let fake = Fake::holding(Some(id(Some("u-a"), None)), true);
        assert_eq!(bind_before_engine_start(&fake, &id(None, Some("a@beebeeb.io")), true), Err(IDENTITY_UNKNOWN.to_string()));
        assert_eq!(*fake.resets.borrow(), 0);
        assert!(fake.records.borrow().is_empty());
    }

    #[test]
    fn the_same_account_keeps_everything_and_completes_the_record() {
        let fake = Fake::holding(Some(id(None, Some("a@beebeeb.io"))), true);
        assert_eq!(bind_before_engine_start(&fake, &id(Some("u-a"), Some("a@beebeeb.io")), true), Ok(Bound::Kept));
        assert_eq!(*fake.resets.borrow(), 0);
        assert_eq!(fake.records.borrow().as_slice(), &[id(Some("u-a"), Some("a@beebeeb.io"))]);
        assert_eq!(bind_before_engine_start(&fake, &id(None, Some("a@beebeeb.io")), true), Ok(Bound::Kept));
        assert_eq!(fake.records.borrow().len(), 1, "an offline start of the same account rewrites nothing");
    }

    #[test]
    fn adoption_happens_once_and_never_replaces_an_owner() {
        let fake = Fake::holding(None, true);
        assert_eq!(adopt_unbound(&fake, &id(None, Some("a@beebeeb.io"))), Ok(true));
        assert_eq!(adopt_unbound(&fake, &id(None, Some("b@beebeeb.io"))), Ok(false), "a recorded owner is never replaced");
        assert_eq!(fake.owner.borrow().clone(), Some(id(None, Some("a@beebeeb.io"))));
        assert_eq!(adopt_unbound(&Fake::holding(None, false), &id(None, Some("a@beebeeb.io"))), Ok(false), "nothing to adopt");
        assert_eq!(adopt_unbound(&Fake::holding(None, true), &Identity::default()), Ok(false), "no identity, no owner");
    }
}
```

In `lib.rs`, next to the other `mod` lines: `mod account_binding;`.

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib account_binding::tests > $EVID/t10-red-pure.log 2>&1; echo "rc=$?"; grep -E "panicked|not yet implemented|test result:" $EVID/t10-red-pure.log | head
```

Expected: 7 tests, each panicking `not yet implemented: step 3`. Paste into Notes.

- [ ] **Step 3: Implement the pure module**

Replace the four `todo!` bodies:

```rust
pub fn same_account(a: &Identity, b: &Identity) -> Option<bool> {
    if let (Some(x), Some(y)) = (&a.user_id, &b.user_id) {
        return Some(x == y);
    }
    match (&a.email, &b.email) {
        (Some(x), Some(y)) => Some(x.eq_ignore_ascii_case(y)),
        _ => None,
    }
}

pub fn decide(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> Binding {
    match owner.filter(|owner| owner.is_known()) {
        Some(owner) => match same_account(owner, session) {
            Some(true) => Binding::Proceed,
            Some(false) => Binding::Reset,
            None => Binding::Refuse,
        },
        None if has_local_data => Binding::Reset,
        None => Binding::Proceed,
    }
}

pub fn bind_before_engine_start(data: &dyn LocalData, session: &Identity, reset_allowed: bool) -> Result<Bound, String> {
    let owner = data.owner()?.filter(Identity::is_known);
    let has_local_data = data.has_account_data()?;
    match decide(owner.as_ref(), session, has_local_data) {
        Binding::Proceed => {
            // The same account, or nothing here yet: keep everything and keep the record current
            // (an id learned later completes an email-only record; a changed email replaces the old).
            let current = Identity {
                user_id: session.user_id.clone().or_else(|| owner.as_ref().and_then(|o| o.user_id.clone())),
                email: session.email.clone().or_else(|| owner.as_ref().and_then(|o| o.email.clone())),
            };
            if current.is_known() && owner.as_ref() != Some(&current) {
                data.record_owner(&current)?;
            }
            Ok(Bound::Kept)
        }
        Binding::Reset if !reset_allowed => Err(OTHER_ACCOUNT_ON_WINDOWS.to_string()),
        Binding::Reset => {
            data.reset()?;
            if session.is_known() {
                data.record_owner(session)?;
            }
            Ok(Bound::Reset)
        }
        Binding::Refuse => Err(IDENTITY_UNKNOWN.to_string()),
    }
}

pub fn adopt_unbound(data: &dyn LocalData, keychain_account: &Identity) -> Result<bool, String> {
    if !keychain_account.is_known() || data.owner()?.is_some_and(|owner| owner.is_known()) || !data.has_account_data()? {
        return Ok(false);
    }
    data.record_owner(keychain_account)?;
    Ok(true)
}
```

Run step 2's command into `$EVID/t10-green-pure.log`. Expected: `test result: ok. 7 passed; 0 failed`.

- [ ] **Step 4: `state_db.rs`: the owner record, the account tables, and the sign-out purge**

Tests first, at the end of `state_db.rs`'s `mod tests`:

```rust
    // ── R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it ──

    #[test]
    fn the_owner_round_trips_and_a_sign_out_purge_forgets_it_with_the_staged_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.owner().unwrap(), None);
        assert!(!db.has_account_data().unwrap(), "a new database holds no account data");
        let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
        db.set_owner(&owner).unwrap();
        assert_eq!(db.owner().unwrap(), Some(owner));
        assert!(!db.has_account_data().unwrap(), "the owner record alone is not account data");
        db.track_staged_payload("/tmp/bb-r10-staged-1.bin", None, false).unwrap();
        assert!(db.has_account_data().unwrap());
        let purge = db.purge_all_local_state().unwrap();
        assert!(purge.payload_paths.contains(&"/tmp/bb-r10-staged-1.bin".to_string()), "staged files are returned for deletion");
        let staged: i64 = db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM staged_payloads", [], |r| r.get(0)).unwrap();
        assert_eq!(staged, 0, "staged payload rows are gone");
        assert_eq!(db.owner().unwrap(), None, "a sign-out forgets the owner");
    }

    #[test]
    fn clearing_account_data_leaves_no_row_and_no_owner() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.0.lock().unwrap().execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", []).unwrap();
        db.track_staged_payload("/tmp/bb-r10-staged-2.bin", None, true).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None)).unwrap();
        assert!(db.has_account_data().unwrap(), "a sync cursor is account data");
        db.clear_account_data().unwrap();
        assert!(!db.has_account_data().unwrap());
        assert_eq!(db.owner().unwrap(), None);
    }

    #[test]
    fn every_table_is_classified_for_the_account_binding() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
            .unwrap();
        let tables: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_>>().unwrap();
        assert!(tables.len() >= 12, "{tables:?}");
        for table in &tables {
            assert!(
                ACCOUNT_TABLES.contains(&table.as_str()) || DEVICE_TABLES.contains(&table.as_str()) || table == "sync_state",
                "{table}: list it in ACCOUNT_TABLES (R10), or in DEVICE_TABLES if it holds no account data"
            );
        }
    }
```

If the classification test names a table that is not in the lists below, add it to `ACCOUNT_TABLES`, unless it provably holds no account data. Then it goes in `DEVICE_TABLES`, with a Notes line for the security review.

RED: `cargo test --locked -p beebeeb-desktop --lib state_db::tests` into `$EVID/t10-red-state-db.log`. Expected: compile errors (`no method named owner`, `cannot find value ACCOUNT_TABLES`).

Implement. Module level, after the imports:

```rust
/// R10 (spec 2026-10-06 §5.6): every table holding one account's data. `has_account_data` counts
/// them and `clear_account_data` empties them. `every_table_is_classified_for_the_account_binding`
/// fails when a new table is not listed here or in `DEVICE_TABLES`.
const ACCOUNT_TABLES: [&str; 10] = [
    "files",
    "operation_queue",
    "local_activity",
    "transfer_activity",
    "fp_changes",
    "fp_sync_anchor",
    "fp_materialized",
    "upload_finalizations",
    "staged_payloads",
    "upload_resume",
];
/// Per-device tables: not counted as account data, emptied by a reset anyway.
const DEVICE_TABLES: [&str; 1] = ["bandwidth_samples"];
const OWNER_USER_ID_KEY: &str = "owner_user_id";
const OWNER_EMAIL_KEY: &str = "owner_email";
```

In `impl StateDb`:

```rust
    /// R10: the account this local data belongs to, if one is recorded.
    pub fn owner(&self) -> Result<Option<crate::account_binding::Identity>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let read = |key: &str| {
            conn.query_row("SELECT value FROM sync_state WHERE key = ?1", params![key], |row| row.get::<_, String>(0))
                .optional()
        };
        let owner = crate::account_binding::Identity::new(read(OWNER_USER_ID_KEY)?.as_deref(), read(OWNER_EMAIL_KEY)?.as_deref());
        Ok(owner.is_known().then_some(owner))
    }

    /// R10: record the owner (both fields at once; an unknown field is removed).
    pub fn set_owner(&self, owner: &crate::account_binding::Identity) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        for (key, value) in [(OWNER_USER_ID_KEY, &owner.user_id), (OWNER_EMAIL_KEY, &owner.email)] {
            match value {
                Some(value) => tx.execute(
                    "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?,
                None => tx.execute("DELETE FROM sync_state WHERE key = ?1", params![key])?,
            };
        }
        tx.commit()
    }

    /// R10: does anything of an account live here, apart from the owner record itself?
    pub fn has_account_data(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sync_state WHERE key NOT IN (?1, ?2)",
            params![OWNER_USER_ID_KEY, OWNER_EMAIL_KEY],
            |row| row.get(0),
        )?;
        for table in ACCOUNT_TABLES {
            rows += conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))?;
        }
        Ok(rows > 0)
    }

    /// R10 reset, after `purge_all_local_state` has handed back the files to delete: every row of the
    /// previous account, the sync cursor and the owner record included, in one transaction.
    pub fn clear_account_data(&self) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()) {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute("DELETE FROM sync_state", [])?;
        tx.commit()
    }
```

In `purge_all_local_state`, make `payload_paths` `let mut`, and directly after `tx.execute("DELETE FROM transfer_activity", [])?;` add:

```rust
        // R10 (spec 2026-10-06 §5.6): staged payloads go with the queue. Their files are returned
        // with the other payloads, so the caller deletes them through the same safety gate.
        let staged_paths: Vec<String> = {
            let mut stmt = tx.prepare("SELECT path FROM staged_payloads")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.execute("DELETE FROM staged_payloads", [])?;
        for path in staged_paths {
            if !payload_paths.contains(&path) {
                payload_paths.push(path);
            }
        }
        // R10: a sign-out forgets which account this local data belonged to.
        tx.execute("DELETE FROM sync_state WHERE key IN (?1, ?2)", params![OWNER_USER_ID_KEY, OWNER_EMAIL_KEY])?;
```

GREEN: the same command into `$EVID/t10-green-state-db.log`. Expected: `0 failed`, with the old `state_db::tests` count + 3.

- [ ] **Step 5: The `lib.rs` tests (RED)**

Extend `mod account_binding_tests` (step 1) with:

```rust
    use crate::account_binding::{Bound, Identity, bind_before_engine_start};

    const RESET_ALLOWED: bool = !cfg!(target_os = "windows");

    fn alice() -> Identity {
        Identity::new(Some("u-a"), Some("a@beebeeb.io"))
    }

    fn bob() -> Identity {
        Identity::new(Some("u-b"), Some("b@beebeeb.io"))
    }

    struct LocalFixture {
        data: StateDbLocalData,
        files: Vec<PathBuf>,
        staging: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Drop for LocalFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }

    /// A computer holding one account's queued change, staged upload and cached file, in real
    /// files under the OS temp dir so `is_disposable_cache_path` allows their removal.
    fn local_data_of(owner: Option<Identity>) -> LocalFixture {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let staging = std::env::temp_dir().join(format!("bb-test-r10-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&staging).unwrap();
        let payload = staging.join("op-1-payload.bin");
        let cache = staging.join("file-a-cache.bin");
        let staged = staging.join("staged-1.bin");
        for path in [&payload, &cache, &staged] {
            std::fs::write(path, b"local bytes").unwrap();
        }
        db.upsert_file(&crate::state_db::FileEntry {
            file_id: "file-a".into(),
            path: "/A.txt".into(),
            status: crate::state_db::FileStatus::Local,
            size_bytes: 11,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::File,
        })
        .unwrap();
        db.mark_cached("file-a", cache.to_str().unwrap(), 11, 10).unwrap();
        db.enqueue_operation(&crate::state_db::PendingOperation {
            op_id: "op-1".into(),
            kind: crate::state_db::OperationKind::UploadVersion,
            file_id: Some("file-a".into()),
            parent_id: None,
            target_path: Some("/A.txt".into()),
            metadata_json: None,
            payload_path: Some(payload.to_str().unwrap().to_string()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
        db.track_staged_payload(staged.to_str().unwrap(), None, false).unwrap();
        if let Some(owner) = owner {
            db.set_owner(&owner).unwrap();
        }
        LocalFixture { data: StateDbLocalData { db, sync_root: None }, files: vec![payload, cache, staged], staging, _dir: dir }
    }

    /// R10: a different account never reuses this computer's local data. macOS and Linux reset it
    /// before the engine starts; Windows starts nothing and discards nothing silently.
    #[test]
    fn a_different_account_never_reuses_local_data() {
        let local = local_data_of(Some(alice()));
        let result = bind_before_engine_start(&local.data, &bob(), RESET_ALLOWED);
        let db = &local.data.db;
        if RESET_ALLOWED {
            assert_eq!(result, Ok(Bound::Reset));
            assert!(db.list_due_operations(i64::MAX).unwrap().is_empty(), "no queued change survives");
            assert!(db.list_files().unwrap().is_empty(), "no file row survives");
            assert!(!db.has_account_data().unwrap());
            for path in &local.files {
                assert!(!path.exists(), "{} is deleted", path.display());
            }
            assert_eq!(db.owner().unwrap(), Some(bob()));
        } else {
            assert_eq!(result, Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string()));
            assert_eq!(db.list_due_operations(i64::MAX).unwrap().len(), 1, "nothing discarded");
            assert_eq!(db.owner().unwrap(), Some(alice()));
        }
    }

    /// R8 + R10: the same account keeps everything, also offline (by email, in any case).
    #[test]
    fn the_same_account_never_purges() {
        for session in [alice(), Identity::new(None, Some("A@Beebeeb.io"))] {
            let local = local_data_of(Some(alice()));
            assert_eq!(bind_before_engine_start(&local.data, &session, RESET_ALLOWED), Ok(Bound::Kept), "{session:?}");
            assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
            for path in &local.files {
                assert!(path.exists(), "{} is kept", path.display());
            }
            assert_eq!(local.data.db.owner().unwrap().and_then(|owner| owner.user_id), Some("u-a".to_string()));
        }
    }

    /// Fail closed: local data without a recorded owner is never handed to a session.
    #[test]
    fn local_data_without_an_owner_is_reset_before_any_account_uses_it() {
        let local = local_data_of(None);
        let result = bind_before_engine_start(&local.data, &bob(), RESET_ALLOWED);
        if RESET_ALLOWED {
            assert_eq!(result, Ok(Bound::Reset));
            assert!(local.data.db.list_due_operations(i64::MAX).unwrap().is_empty());
            for path in &local.files {
                assert!(!path.exists(), "{} is deleted", path.display());
            }
            assert_eq!(local.data.db.owner().unwrap(), Some(bob()));
        } else {
            assert_eq!(result, Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string()));
            assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1);
        }
    }

    #[test]
    fn a_fresh_computer_records_its_first_account() {
        let dir = tempfile::tempdir().unwrap();
        let data = StateDbLocalData { db: crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap(), sync_root: None };
        assert_eq!(bind_before_engine_start(&data, &bob(), RESET_ALLOWED), Ok(Bound::Kept));
        assert_eq!(data.db.owner().unwrap(), Some(bob()));
    }

    #[test]
    fn the_session_identity_takes_the_profile_only_when_it_is_this_sessions() {
        let profile = |email: &str| {
            serde_json::from_str::<account_dto::AccountProfile>(&format!(
                r#"{{"user_id":"u-a","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
            ))
            .unwrap()
        };
        let acct = crate::account::AccountRuntime::new(crate::account::AccountId::new_v4());
        *acct.auth_email.lock().unwrap() = Some("a@beebeeb.io".into());
        *acct.cached_profile.lock().unwrap() = Some(profile("A@beebeeb.io"));
        assert_eq!(session_identity(&acct), Identity::new(Some("u-a"), Some("a@beebeeb.io")));
        *acct.cached_profile.lock().unwrap() = Some(profile("someone-else@beebeeb.io"));
        assert_eq!(session_identity(&acct), Identity::new(None, Some("a@beebeeb.io")), "a profile of another email is ignored");
        *acct.cached_profile.lock().unwrap() = None;
        assert_eq!(session_identity(&acct), Identity::new(None, Some("a@beebeeb.io")), "offline: the email alone");
    }

    /// R10: on macOS and Linux a sign-out whose purge fails stops, instead of warning and completing.
    #[test]
    fn a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let clear = body_of(production(&source), "async fn clear_session_impl(");
        let purge = &clear[clear.find("match state_db_from_app_local_state_dir() {").expect("the non-Windows purge")..];
        let purge = &purge[..purge.find("\n    }\n").expect("the end of the purge match")];
        assert_eq!(purge.matches("return Err(").count(), 2, "a failed purge and an unreadable database both stop it");
        assert!(!purge.contains("tracing::warn!"), "no warn-and-continue left");
        assert!(purge.contains("SIGN_OUT_PURGE_FAILED"));
    }

    /// The upgrade path runs before the probe can drop a revoked token and before any engine starts.
    #[test]
    fn the_startup_restore_adopts_unbound_local_data_before_anything_starts() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let restore = body_of(production(&source), "async fn restore_session_on_startup(");
        let adopt = restore.find("adopt_unbound_local_data_at_startup(&acct);").expect("the upgrade path runs at startup");
        assert!(adopt < restore.find("probe_startup_session(").expect("the probe"));
        assert!(adopt < restore.find("start_engine_if_possible(").expect("the engine start"));
    }
```

RED: `cargo test --locked -p beebeeb-desktop --lib account_binding_tests` into `$EVID/t10-red-lib.log`. Expected: compile errors (`cannot find struct StateDbLocalData`, `cannot find function session_identity`). Paste them into Notes.

- [ ] **Step 6: Implement in `lib.rs`**

Next to `state_db_from_state_dir`:

```rust
/// R10 (spec 2026-10-06 §5.6): the binding's view of this computer's local data.
struct StateDbLocalData {
    db: state_db::StateDb,
    sync_root: Option<PathBuf>,
}

impl StateDbLocalData {
    /// Opens `state.db` the way the engine will, and creates it if needed, so the owner of a first
    /// engine start is recorded before that engine writes anything. On a first start the state-dir
    /// migration runs here first; the runner's own call then finds `state.db` and skips it.
    fn for_engine_start(sync_root: &Path) -> Result<Self, String> {
        let state_dir = state_paths::beebeeb_state_dir()?;
        let path = state_paths::state_db_path_from_state_dir(&state_dir);
        if !path.exists() {
            state_paths::prepare_beebeeb_state_dir_at(sync_root, &state_dir)?;
        }
        let db = state_db::StateDb::open(&path).map_err(|e| format!("open state.db: {e}"))?;
        Ok(Self { db, sync_root: Some(sync_root.to_path_buf()) })
    }
}

impl account_binding::LocalData for StateDbLocalData {
    fn owner(&self) -> Result<Option<account_binding::Identity>, String> {
        self.db.owner().map_err(|e| format!("read the local data owner: {e}"))
    }

    fn has_account_data(&self) -> Result<bool, String> {
        self.db.has_account_data().map_err(|e| format!("inspect the local data: {e}"))
    }

    fn reset(&self) -> Result<(), String> {
        let summary = purge_local_state_files(&self.db, self.sync_root.as_deref())?;
        tracing::info!(
            queued_ops_purged = summary.queued_ops_purged,
            files_removed = summary.files_removed,
            files_skipped = summary.files_skipped,
            "R10: local data reset before sync started"
        );
        self.db.clear_account_data().map_err(|e| format!("clear the local data: {e}"))
    }

    fn record_owner(&self, owner: &account_binding::Identity) -> Result<(), String> {
        self.db.set_owner(owner).map_err(|e| format!("record the local data owner: {e}"))
    }
}

/// R10: who the session about to sync is. The profile its sign-in fetched, when that profile has
/// this session's email; otherwise the email alone (an offline relaunch).
fn session_identity(acct: &crate::account::AccountRuntime) -> account_binding::Identity {
    let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone()).filter(|profile| {
        email.as_deref().is_none_or(|email| email.trim().eq_ignore_ascii_case(profile.email.trim()))
    });
    account_binding::Identity::new(
        profile.as_ref().map(|profile| profile.user_id.as_str()),
        email.as_deref().or(profile.as_ref().map(|profile| profile.email.as_str())),
    )
}

/// R10: bind this computer's local data to the session's account before an engine starts. Another
/// account's data is reset first (refused on Windows). An owner that cannot be compared stops the
/// start and deletes nothing. Any error means: do not start the engine.
fn bind_local_data_to_session(
    state: &AppState,
    acct: &crate::account::AccountRuntime,
    sync_root: &Path,
) -> Result<(), String> {
    let data = StateDbLocalData::for_engine_start(sync_root)?;
    let session = session_identity(acct);
    match account_binding::bind_before_engine_start(&data, &session, !cfg!(target_os = "windows"))? {
        account_binding::Bound::Kept => {}
        account_binding::Bound::Reset => {
            tracing::warn!("R10: local data of another account was reset before sync started");
            // Finder may still list what the reset removed. A Repair removes Beebeeb and adds it back.
            // Never awaited: the reconciler itself starts engines through this function.
            #[cfg(target_os = "macos")]
            notify_finder(state, finder_setup::core::Trigger::Repair);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = state;
    Ok(())
}

/// R10 (spec 2026-10-06 §5.6): the only place a sync engine starts. The local data is bound to the
/// session's account first, and on any error nothing starts. `every_engine_start_is_bound_first`
/// pins both.
#[allow(clippy::too_many_arguments)]
fn spawn_bound_engine(
    app: tauri::AppHandle,
    state: &AppState,
    acct: &crate::account::AccountRuntime,
    sync_root: PathBuf,
    token: String,
    master_key: [u8; 32],
    sync_paused: Arc<AtomicBool>,
    auth_health: Arc<runner::AuthHealth>,
) -> Result<EngineRunner, String> {
    bind_local_data_to_session(state, acct, &sync_root)?;
    Ok(EngineRunner::spawn(app, sync_root, token, master_key, sync_paused, auth_health))
}

/// R10, the upgrade path: local data from before the binding has no owner. The account this
/// computer's Keychain still names has been using it, so it becomes the owner before anything
/// starts. Best-effort: without it, the data is reset at the next engine start (fail closed).
fn adopt_unbound_local_data_at_startup(acct: &crate::account::AccountRuntime) {
    let Ok(Some(db)) = state_db_from_app_local_state_dir() else {
        return;
    };
    let data = StateDbLocalData { db, sync_root: None };
    let keychain_account = account_binding::Identity::new(None, keychain_account_email(acct.id.as_str()).as_deref());
    match account_binding::adopt_unbound(&data, &keychain_account) {
        Ok(true) => tracing::info!("R10: existing local data recorded as this computer's signed-in account's"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "R10: could not record the owner of existing local data"),
    }
}
```

(`Arc`, `AtomicBool`, `Path` and `PathBuf` are already imported at the top of `lib.rs`. If `state_paths::beebeeb_state_dir()`'s error type is not `String`, add `.map_err(|e| e.to_string())`.)

The five engine starts. Replace each production `EngineRunner::spawn(` call with `spawn_bound_engine(`. Insert two arguments after `app`: the function's `AppState` (`state`, `&state` or `&*state`, as its type needs) and its account (`&acct`). Add `?` after the call:

1. `start_engine_if_possible`: `*engine_slot = Some(EngineRunner::spawn(app, root, token, master_key, pause_flag, auth_health));` becomes `*engine_slot = Some(spawn_bound_engine(app, state, &acct, root, token, master_key, pause_flag, auth_health)?);`.
2. `persist_sync_root_and_start_engine`, `start_engine_for_pending_finder_install`, `pick_sync_root`, and Task 8's `ensure_sync_root_and_engine`: the same change. In `start_engine_for_pending_finder_install`, `let runner = EngineRunner::spawn(…);` becomes `let runner = spawn_bound_engine(…)?;`.

Keep `EngineRunner::spawn(` out of every comment in `lib.rs`. The census counts the text.

`restore_session_on_startup`: directly after the `if acct.session.lock().map(|g| g.is_some()).unwrap_or(false) { return; }` block, add:

```rust
    // R10: local data from before the account binding gets its owner before anything can start.
    adopt_unbound_local_data_at_startup(&acct);
```

`clear_session_impl`, the non-Windows purge. Above the `match state_db_from_app_local_state_dir() {`, add the constant at module level:

```rust
/// R10: on macOS and Linux a sign-out whose local-data purge fails stops instead of completing, as
/// Windows already does. The engine is already stopped, so trying again is safe.
#[cfg(not(target_os = "windows"))]
const SIGN_OUT_PURGE_FAILED: &str =
    "Sign-out paused: Beebeeb couldn’t clear this computer’s local data. Try again; if it keeps happening, restart Beebeeb.";
```

Replace the two warn arms of that match:

```rust
            Err(error) => {
                tracing::error!(error = %error, "sign-out stopped: the local data could not be purged");
                return Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"));
            }
```

```rust
        Err(error) => {
            tracing::error!(error = %error, "sign-out stopped: the local state database could not be opened");
            return Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"));
        }
```

In the comment above that block, replace the sentence "Best-effort and non-fatal per file: logout must always appear to succeed, and the DB rows are already cleared …" with "Per file it stays best-effort (the DB rows are cleared regardless). A failure of the purge itself stops the sign-out (R10, spec 2026-10-06 §5.6)." If an existing test expected a failed purge to complete, it encoded the old policy: change its expectation and write that into Notes.

- [ ] **Step 7: GREEN, the whole crate**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib account_binding > $EVID/t10-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t10-green.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib purge_local_state_files > $EVID/t10-purge.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t10-purge.log
$LOCK cargo-build -- cargo test --locked > $EVID/t10-all.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t10-all.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t10-check.log 2>&1; echo "rc=$?"
```

Expected:
- The `account_binding` filter: `ok. 15 passed; 0 failed` (7 in `account_binding::tests`, 8 in `account_binding_tests`).
- The existing `purge_local_state_files_*` tests still pass.
- Every binary of the full run, including `keychain`, `windows_session_wiring` and `windows_signout_cleanup`, reports `0 failed`.
- `cargo check` gives `rc=0`.

The `cfg!(target_os = "windows")` branches run only on CI's Windows job. Before Lane R merges, the lead reads that job's `test result:` line for `account_binding`. The PR says so.

- [ ] **Step 8: Mutation checks (required for R10)**

Make each change, run the step-7 `account_binding` command, confirm the named test fails, then restore:

1. In `spawn_bound_engine`, change `bind_local_data_to_session(state, acct, &sync_root)?;` to `let _ = bind_local_data_to_session(state, acct, &sync_root);`. Expected: `every_engine_start_is_bound_first` fails (`a failed binding returns before the spawn`).
2. In `pick_sync_root`, put back `EngineRunner::spawn(` (without the two extra arguments and the `?`). Expected: `every_engine_start_is_bound_first` fails (`left: 2`).
3. In `decide`, `None if has_local_data => Binding::Reset` → `Binding::Proceed`. Expected: `the_r10_table` and `local_data_without_an_owner_is_reset_before_any_account_uses_it` fail.
4. In `decide`, `Some(false) => Binding::Reset` → `Binding::Proceed`. Expected: `a_different_account_never_reuses_local_data` fails.
5. In `decide`, `Some(true) => Binding::Proceed` → `Binding::Reset`. Expected: `the_same_account_never_purges` fails.
6. In `bind_before_engine_start`, `data.reset()?;` → `let _ = data.reset();`. Expected: `a_failed_reset_blocks_the_engine_and_records_nothing` fails.
7. In `purge_all_local_state`, delete `tx.execute("DELETE FROM staged_payloads", [])?;`. Expected (with the `state_db::tests` filter): `the_owner_round_trips_and_a_sign_out_purge_forgets_it_with_the_staged_payloads` fails.
8. In `clear_session_impl`, put back the warn-only arm for a failed purge. Expected: `a_failed_sign_out_purge_stops_the_sign_out_on_macos_and_linux` fails.
9. Move `adopt_unbound_local_data_at_startup(&acct);` below the probe. Expected: `the_startup_restore_adopts_unbound_local_data_before_anything_starts` fails.

Paste every failure into Notes, and the failures of 1, 4 and 8 also into the private task's Notes.

- [ ] **Step 9: Commit (neutral message)**

```bash
cd $WT && git add src-tauri/src/account_binding.rs
git commit -m "sync: local data is bound to the account that created it; every engine start checks it first

A different account never reuses this computer's local data. A failed
reset or sign-out purge stops instead of continuing. Windows refuses
instead of resetting.

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/account_binding.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
git show --stat HEAD
```

`git show --stat HEAD` lists exactly 3 files.

---

## Task 11: Re-sign-in in place: the account check and the token swap (R8, backend)

**Lane R.** Ruling R8: "Sign in again as the same account and only the session token is replaced. Finder, keys, cache and pending edits all stay, and the edits upload afterwards. Signing in as a different account is an account switch: full sign-out, with a warning if edits haven't uploaded yet." This task is the backend half. Task 18 is the frontend. It reads and records the owner of the local data through Task 10's `StateDb::owner`/`set_owner`. **Security-sensitive**: step 9 is the mandatory security review, and it covers Tasks 10–12.

**Scope rule: Windows is unchanged.** On Windows, `desktop_login`, `desktop_login_2fa` and `apply_session` keep refusing while a session is present ("Sign out of the current account before signing in again.", 4 occurrences in `lib.rs` at `c351f98`), and the account check is compiled out there (`#[cfg(not(target_os = "windows"))]`). The account check runs on macOS **and Linux**, because Linux shares `Onboarding.tsx` and the same commands. On Linux, `forceReauth` still clears first (Task 18), so a Linux re-sign-in through the banner is always `Fresh`. After a startup 401 (Task 12 keeps the key on Linux too) it can be `SameAccount`, which reloads the kept key. The account binding of Task 10 applies there as everywhere.

**Files:**
- Create: `src-tauri/src/reauth.rs` (pure)
- Modify: `src-tauri/src/lib.rs`: `mod reauth;`, `LoginOutcome`, `desktop_login`, `desktop_login_2fa`, new helpers and the `open_reauth_window` command, tests, and the census in `finder_setup_wiring_tests` (Task 9)
- Modify: `src-tauri/src/browser_login.rs` (`run_handoff`: the same check before `apply_session`)
- Modify: `src-tauri/src/keychain.rs` (`holds_vault_key`, a test) and `src-tauri/src/state_db.rs` (`queued_or_staged_count`, a test): the two probes behind `LocalTraces` (Spec issue 24)

**Interfaces:**
- Consumes: `StateDb::owner`/`set_owner` and `account_binding::Identity` (Task 10); `keys_arrived`, `notify_finder` (Tasks 8, 9); `account_dto::AccountProfile { user_id, email, .. }`; the existing `fetch_session_profile`, `revoke_desktop_session`, `persist_session_token_to_keychain`, `load_session_from_keychain`, `install_unlocked_session`, `start_engine_if_possible`, `keychain_session_present`, `keychain_account_email`, `state_db_from_app_local_state_dir`.
- Produces:
  - `reauth::{LocalTraces { session_in_memory, auth_present, keychain_token, keychain_email, vault_key, recorded_owner, cached_profile, queued_or_staged } (all `bool`; `any() -> bool`), LocalAccount<'a> { user_id: Option<&'a str>, email: Option<&'a str>, traces: bool }, SignInKind { Fresh, SameAccount, DifferentAccount }, sign_in_kind(&LocalAccount, user_id: &str, email: &str) -> SignInKind}`
  - `LoginOutcome { requires_2fa: bool, reauthenticated: bool, vault_unlocked: bool, account_mismatch: Option<AccountMismatchDto { pending_changes: u64 }> }`. JSON example: `{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":3}}`. `desktop_login_2fa` now returns it too (it returned `()`). Windows' frontend reads only `requires_2fa`, so the extra fields change nothing there.
  - `keychain::holds_vault_key<S: AuthSecretStore>(&S) -> bool` and `StateDb::queued_or_staged_count() -> Result<u64>`.
  - Non-Windows: `fn local_data_owner() -> Result<Option<account_binding::Identity>, String>`, `fn keychain_vault_key_present(&str) -> bool`, `fn queued_or_staged_present() -> bool` and `fn record_local_data_owner(&account_binding::Identity) -> Result<(), String>`. Also `fn pending_changes_count() -> u64`, `fn replace_session_token_in_memory(&AccountRuntime, &str) -> Result<bool, String>`, `fn reauth_settle_flags(&AppState, &AccountRuntime, &str)`.
  - Non-Windows only: `enum SignInSettlement { Fresh, Reauthenticated { vault_unlocked: bool }, AccountMismatch { pending_changes: u64 } }`, `async fn settle_sign_in(app, state: &State<'_, AppState>, token: &str, profile: &AccountProfile) -> Result<SignInSettlement, String>`, and `async fn reauth_in_place(app, state, acct: &AccountRuntime, token, email, user_id) -> Result<bool, String>`.
  - Command `open_reauth_window` (non-Windows): opens or reloads the `onboarding` window at `index.html?window=onboarding&mode=reauth`.

- [ ] **Step 1: The pure decision, tests first**

`src-tauri/src/reauth.rs`:

```rust
//! Ruling R8 (spec 2026-10-06 §3): a sign-in on a Mac that already holds an account is either the
//! same account signing in again (only the session token is replaced: Finder, keys, cache and
//! pending edits stay) or an account switch (a full sign-out comes first, after a warning). Pure:
//! the caller gathers the facts; this decides.

/// What this Mac still holds of an account when someone signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalAccount<'a> {
    /// The user id of the recorded owner of this computer's local data (R10, Task 10).
    pub user_id: Option<&'a str>,
    /// The email this Mac recorded for its account (Keychain email, else `last_signed_in_email`).
    /// Consulted only when `user_id` is unknown: an owner recorded by email only (the upgrade
    /// path), or no owner yet (plan "Spec issues" 19).
    pub email: Option<&'a str>,
    /// `LocalTraces::any()`: anything of a previous account is still on this computer.
    pub traces: bool,
}

/// Everything of a previous account that can outlive its session on this computer (Codex P1 on
/// PR #113, plan "Spec issues" 24). A startup 401 keeps the Keychain email and the vault key (R9);
/// an install from before R10 can have an email-only owner. `Fresh` needs none of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTraces {
    pub session_in_memory: bool,
    pub auth_present: bool,
    pub keychain_token: bool,
    pub keychain_email: bool,
    pub vault_key: bool,
    pub recorded_owner: bool,
    pub cached_profile: bool,
    pub queued_or_staged: bool,
}

impl LocalTraces {
    pub fn any(&self) -> bool {
        todo!("Task 11 step 2")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInKind {
    /// Nothing of an account is on this Mac: the ordinary first sign-in.
    Fresh,
    /// The account on this Mac signs in again: replace only the session token.
    SameAccount,
    /// Another account, or one whose identity cannot be established (fail closed): a switch.
    DifferentAccount,
}

pub fn sign_in_kind(local: &LocalAccount<'_>, user_id: &str, email: &str) -> SignInKind {
    todo!("Task 11 step 2")
}

#[cfg(test)]
mod tests {
    use super::*;
    use SignInKind::*;

    fn local(user_id: Option<&'static str>, email: Option<&'static str>, traces: bool) -> LocalAccount<'static> {
        LocalAccount { user_id, email, traces }
    }

    #[test]
    fn the_r8_table() {
        let cases: Vec<(&str, LocalAccount<'static>, SignInKind)> = vec![
            ("a fresh Mac", local(None, None, false), Fresh),
            ("signed out by choice earlier (only the prefill email is left)", local(None, Some("sam@beebeeb.io"), false), Fresh),
            ("the same account by id", local(Some("u-1"), Some("sam@beebeeb.io"), true), SameAccount),
            ("another account by id", local(Some("u-2"), Some("sam@beebeeb.io"), true), DifferentAccount),
            ("an id without other traces still decides", local(Some("u-2"), None, false), DifferentAccount),
            ("pre-R8 install, same email, any case and spacing", local(None, Some("  Sam@Beebeeb.IO "), true), SameAccount),
            ("pre-R8 install, another email", local(None, Some("kim@beebeeb.io"), true), DifferentAccount),
            ("pre-R8 install, no identity at all: fail closed", local(None, None, true), DifferentAccount),
            ("an empty recorded email never matches", local(None, Some(""), true), DifferentAccount),
        ];
        for (name, account, expected) in cases {
            assert_eq!(sign_in_kind(&account, "u-1", "sam@beebeeb.io"), expected, "{name}");
        }
    }

    #[test]
    fn an_empty_signing_in_email_never_matches_either() {
        assert_eq!(sign_in_kind(&local(None, Some(""), true), "u-9", ""), DifferentAccount);
    }

    // ── Codex P1 on PR #113 (plan "Spec issues" 24): every retained trace counts ──

    #[test]
    fn every_retained_trace_counts() {
        assert!(!LocalTraces::default().any(), "a computer with nothing left is fresh");
        let each: [(&str, fn(&mut LocalTraces)); 8] = [
            ("a session in memory", |t| t.session_in_memory = true),
            ("auth present", |t| t.auth_present = true),
            ("a Keychain token", |t| t.keychain_token = true),
            ("a Keychain email", |t| t.keychain_email = true),
            ("a vault key", |t| t.vault_key = true),
            ("a recorded owner", |t| t.recorded_owner = true),
            ("a cached profile", |t| t.cached_profile = true),
            ("queued or staged data", |t| t.queued_or_staged = true),
        ];
        for (name, set) in each {
            let mut traces = LocalTraces::default();
            set(&mut traces);
            assert!(traces.any(), "{name} alone is a trace");
        }
    }

    /// An install from before R10 whose token was revoked at startup, with nothing queued: Task 10
    /// adopted an email-only owner, and Task 12 kept the Keychain email and the vault key (R9).
    fn after_a_revoked_token_with_nothing_queued() -> LocalAccount<'static> {
        let traces = LocalTraces { recorded_owner: true, keychain_email: true, vault_key: true, ..LocalTraces::default() };
        LocalAccount { user_id: None, email: Some("sam@beebeeb.io"), traces: traces.any() }
    }

    #[test]
    fn another_account_after_a_revoked_token_with_nothing_queued_is_a_switch_never_fresh() {
        assert_eq!(sign_in_kind(&after_a_revoked_token_with_nothing_queued(), "u-2", "kim@beebeeb.io"), DifferentAccount);
    }

    #[test]
    fn the_same_account_after_a_revoked_token_signs_in_again_in_place() {
        assert_eq!(sign_in_kind(&after_a_revoked_token_with_nothing_queued(), "u-1", "Sam@beebeeb.io"), SameAccount);
    }

    #[test]
    fn a_retained_vault_key_alone_with_no_email_anywhere_is_never_fresh() {
        let traces = LocalTraces { vault_key: true, ..LocalTraces::default() };
        let local = LocalAccount { user_id: None, email: None, traces: traces.any() };
        assert_eq!(sign_in_kind(&local, "u-2", "kim@beebeeb.io"), DifferentAccount);
    }
}
```

Add `mod reauth;` to `lib.rs` next to `mod finder_setup;`.

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib reauth::tests > $EVID/t11-red-pure.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t11-red-pure.log
```

Expected: `test result: FAILED. 0 passed; 6 failed` (every test reaches a `todo!`).

- [ ] **Step 2: Implement `sign_in_kind`**

```rust
pub fn sign_in_kind(local: &LocalAccount<'_>, user_id: &str, email: &str) -> SignInKind {
    if let Some(stored) = local.user_id {
        return if stored == user_id { SignInKind::SameAccount } else { SignInKind::DifferentAccount };
    }
    if !local.traces {
        return SignInKind::Fresh;
    }
    match local.email.map(str::trim) {
        Some(stored) if !stored.is_empty() && stored.eq_ignore_ascii_case(email.trim()) => SignInKind::SameAccount,
        _ => SignInKind::DifferentAccount,
    }
}
```

`LocalTraces::any`. **First reproduce Codex's P1 (PR #113).** Write it with only round 2's four terms:

```rust
    pub fn any(&self) -> bool {
        self.session_in_memory || self.auth_present || self.keychain_token || self.queued_or_staged
    }
```

Run the step 1 command into `$EVID/t11-p1-red.log`. Expected: `FAILED. 2 passed; 4 failed`: `every_retained_trace_counts` (`a Keychain email alone is a trace`) and the three revoked-token tests (`left: Fresh`). That is the P1 reproduced. Paste it into Notes. Then write the real one:

```rust
    pub fn any(&self) -> bool {
        self.session_in_memory
            || self.auth_present
            || self.keychain_token
            || self.keychain_email
            || self.vault_key
            || self.recorded_owner
            || self.cached_profile
            || self.queued_or_staged
    }
```

Run the step 1 command into `$EVID/t11-green-pure.log`. Expected: `ok. 6 passed`. **Mutation (the new `traces` terms, required):** remove `|| self.vault_key`. Expected: `every_retained_trace_counts` fails (`a vault key alone is a trace`), and so does `a_retained_vault_key_alone_with_no_email_anywhere_is_never_fresh` (`left: Fresh`). Restore it. Then remove `|| self.keychain_email`, `|| self.recorded_owner` and `|| self.cached_profile` one at a time. Each makes `every_retained_trace_counts` name it. Paste all four failures. **Mutation:** change the fallback arm `_ => SignInKind::DifferentAccount` to `SameAccount`. Expected: `the_r8_table` fails on "another email" and "no identity at all". Restore it. Paste the failure into Notes.

- [ ] **Step 3: The `lib.rs` tests (RED)**

Add a module after `finder_setup_wiring_tests`:

```rust
#[cfg(test)]
mod reauth_tests {
    use super::*;

    fn source() -> String {
        include_str!("lib.rs").replace("\r\n", "\n")
    }

    fn production(source: &str) -> &str {
        &source[..source.find("#[cfg(test)]\nmod tests {").expect("the main test module")]
    }

    fn body_of(source: &str, signature: &str) -> String {
        let start = source.find(signature).unwrap_or_else(|| panic!("{signature} exists"));
        let rest = &source[start..];
        rest[..rest.find("\n}\n").map(|i| i + 3).unwrap_or(rest.len())].to_string()
    }

    fn unlocked_account(state: &AppState) -> Arc<AccountRuntime> {
        synthesize_single_account(state, AccountId("reauth-test".into()));
        let acct = state.active_account().unwrap();
        *acct.session.lock().unwrap() = Some(Session { token: "old-token".into(), master_key: [7u8; 32], email: Some("sam@beebeeb.io".into()) });
        acct
    }

    #[test]
    fn a_new_token_replaces_only_the_token_in_memory() {
        let state = AppState::default();
        let acct = unlocked_account(&state);
        assert!(replace_session_token_in_memory(&acct, "new-token").unwrap());
        let guard = acct.session.lock().unwrap();
        let session = guard.as_ref().unwrap();
        assert_eq!(session.token, "new-token");
        assert_eq!(session.master_key, [7u8; 32], "the keys stay");
        assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
    }

    #[test]
    fn without_keys_in_memory_nothing_is_invented() {
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("reauth-empty".into()));
        let acct = state.active_account().unwrap();
        assert!(!replace_session_token_in_memory(&acct, "new-token").unwrap());
        assert!(acct.session.lock().unwrap().is_none());
    }

    #[test]
    fn a_re_sign_in_ends_the_revoked_streak_and_never_asks_finder_to_remove() {
        let state = AppState::default();
        let acct = unlocked_account(&state);
        for _ in 0..3 {
            acct.auth_health.note_result(Some(&anyhow::anyhow!("HTTP 401 Unauthorized: session revoked")));
        }
        assert!(acct.auth_health.is_expired(), "precondition: the session reads as revoked");
        let (handle, mut rx) = finder_setup::driver::FinderSetupHandle::for_test(
            finder_setup::driver::FinderSetupView::initial(finder_setup::launch_location::LaunchLocation::Applications),
        );
        let _ = state.finder_setup.set(handle);
        reauth_settle_flags(&state, &acct, "sam@beebeeb.io");
        keys_arrived(&state);
        assert!(!acct.auth_health.is_expired());
        assert!(*state.auth_present.lock().unwrap());
        while let Ok(event) = rx.try_recv() {
            assert!(
                matches!(event, finder_setup::driver::Event::Trigger(finder_setup::core::Trigger::KeysArrived)),
                "a re-sign-in only ever tells Finder that keys are here"
            );
        }
    }

    /// R8: "Finder, keys, cache and pending edits all stay". The re-sign-in path never purges,
    /// never removes the domain, never writes or clears keys, and never signs out.
    #[test]
    fn a_re_sign_in_never_purges_removes_or_touches_keys() {
        let source = source();
        let production = production(&source);
        for signature in [
            "async fn settle_sign_in(",
            "async fn reauth_in_place(",
            "fn replace_session_token_in_memory(",
            "fn reauth_settle_flags(",
        ] {
            let body = body_of(production, signature);
            for forbidden in [
                "purge_local_state_files",
                "purge_all_local_state",
                "clear_keychain_session",
                "clear_session_impl",
                "remove_file_provider_domain",
                "finder_remove_for",
                "Trigger::SignOut",
                "persist_vault_key_to_keychain",
                "persist_session_to_keychain",
                "store_wrapped_master_key",
                "master_key =",
            ] {
                assert!(!body.contains(forbidden), "{signature} must not call {forbidden}");
            }
        }
    }

    /// A different account changes nothing local: the mismatch arms only revoke the new session
    /// and return the warning.
    #[test]
    fn a_mismatch_changes_nothing_on_this_mac() {
        let source = source();
        let production = production(&source);
        let settle = body_of(production, "async fn settle_sign_in(");
        assert!(settle.contains(
            "reauth::SignInKind::DifferentAccount => Ok(SignInSettlement::AccountMismatch { pending_changes }),"
        ));
        let browser = include_str!("browser_login.rs").replace("\r\n", "\n");
        for text in [body_of(production, "async fn desktop_login("), body_of(production, "async fn desktop_login_2fa("), browser] {
            let arm = &text[text.find("SignInSettlement::AccountMismatch { pending_changes } => {").expect("a mismatch arm")..];
            let arm = &arm[..arm.find("\n            }\n").or_else(|| arm.find("\n        }\n")).expect("arm end")];
            for forbidden in ["persist_", "record_local_data_owner", "set_auth_", "clear_", "acct.session", "cached_profile", "apply_session"] {
                assert!(!arm.contains(forbidden), "the mismatch arm must not touch {forbidden}:\n{arm}");
            }
            assert!(arm.contains("revoke_desktop_session("), "the session just minted for the other account is revoked");
        }
    }

    /// Every sign-in method goes through the same account check, before anything is stored, and
    /// Windows keeps its refusal and never reaches the check.
    #[test]
    fn every_sign_in_method_is_checked_and_windows_is_unchanged() {
        let source = source();
        let production = production(&source);
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let body = body_of(production, signature);
            let check = body.find("settle_sign_in(").unwrap_or_else(|| panic!("{signature} checks the account"));
            let store = body.find("persist_session_token_to_keychain(").expect("then stores the token");
            assert!(check < store, "{signature}: the check comes before anything is stored");
            let cfg_at = body[..check].rfind("#[cfg(not(target_os = \"windows\"))]").unwrap_or(0);
            assert!(cfg_at > 0 && check - cfg_at < 120, "{signature}: the check sits directly under a not-Windows cfg");
        }
        let browser = include_str!("browser_login.rs").replace("\r\n", "\n");
        let check = browser.find("crate::settle_sign_in(").expect("the browser handoff checks the account");
        assert!(check < browser.find("crate::apply_session(").expect("then applies the session"));
        assert_eq!(
            production.matches("Sign out of the current account before signing in again.").count(),
            4,
            "Windows keeps all four refusals"
        );
    }

    /// R10: a same-account re-sign-in records the owner before anything else changes.
    #[test]
    fn a_same_account_re_sign_in_records_the_owner_first() {
        let source = source();
        let body = body_of(production(&source), "async fn reauth_in_place(");
        let record = body.find("record_local_data_owner(").expect("the owner is recorded");
        assert!(record < body.find("persist_session_token_to_keychain(").expect("then the token"));
    }

    /// Codex P1 on PR #113: the account check looks at every retained trace, not only the token.
    #[test]
    fn settle_sign_in_counts_every_retained_trace() {
        let source = source();
        let settle = body_of(production(&source), "async fn settle_sign_in(");
        for probe in [
            "acct.session.lock()",
            "state.auth_present.lock()",
            "keychain_session_present(",
            "keychain_account_email(",
            "keychain_vault_key_present(",
            "local_data_owner()",
            "acct.cached_profile.lock()",
            "queued_or_staged_present()",
            "traces: traces.any()",
        ] {
            assert!(settle.contains(probe), "settle_sign_in must use {probe}");
        }
    }
}
```

Extend the census in `finder_setup_wiring_tests::every_function_that_installs_keys_tells_the_finder_reconciler` (Task 9): the expected installer list is unchanged (`reauth_in_place` installs through `install_unlocked_session`), but replace the assertion `production.matches("install_unlocked_session(&acct, session)").count() == 1` with:

```rust
        // install_unlocked_session's production callers: unlock_vault and reauth_in_place (R8).
        assert_eq!(production.matches("install_unlocked_session(").count() - 1, 2, "callers, not counting its definition");
```

and add `("async fn reauth_in_place(", 1)` to the `(signature, engine_starts)` list.

RED:

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib reauth_tests > $EVID/t11-red.log 2>&1; echo "rc=$?"; grep -E "error\[|cannot find" $EVID/t11-red.log | head
```

Expected: compile errors (`cannot find function replace_session_token_in_memory`, `reauth_settle_flags`). Paste them into Notes.

- [ ] **Step 4: The helpers and the outcome type**

In `lib.rs`, next to `keys_arrived`:

```rust
/// R8 + R10: the recorded owner of this computer's local data: the account R8 compares.
#[cfg(not(target_os = "windows"))]
fn local_data_owner() -> Result<Option<account_binding::Identity>, String> {
    match state_db_from_app_local_state_dir()? {
        Some(db) => db.owner().map_err(|e| format!("read the local data owner: {e}")),
        None => Ok(None),
    }
}

/// Codex P1 on PR #113: does the Keychain still hold a vault key for this account slot, in the
/// id-keyed or the legacy store? It never unlocks, and the bytes are dropped unread. Fails closed.
#[cfg(not(target_os = "windows"))]
fn keychain_vault_key_present(account_id: &str) -> bool {
    keychain::holds_vault_key(&platform_keychain_store_for(account_id))
        || keychain::holds_vault_key(&keychain::legacy_platform_keychain_store())
}

/// Codex P1 on PR #113: changes waiting to upload, or staged copies of them, in the local data.
/// An unreadable `state.db` counts as present (fail closed).
#[cfg(not(target_os = "windows"))]
fn queued_or_staged_present() -> bool {
    match state_db_from_app_local_state_dir() {
        Ok(Some(db)) => db.queued_or_staged_count().map(|n| n > 0).unwrap_or(true),
        Ok(None) => false,
        Err(_) => true,
    }
}

/// R8 + R10: the same account signed in again (decided by `reauth::sign_in_kind`), so it owns the
/// local data. Never called for another account.
#[cfg(not(target_os = "windows"))]
fn record_local_data_owner(owner: &account_binding::Identity) -> Result<(), String> {
    match state_db_from_app_local_state_dir()? {
        Some(db) => db.set_owner(owner).map_err(|e| format!("record the local data owner: {e}")),
        None => Ok(()),
    }
}

/// How many changes on this Mac have not uploaded yet: the number in the R8 warning.
fn pending_changes_count() -> u64 {
    match state_db_from_app_local_state_dir() {
        Ok(Some(db)) => db
            .queue_diagnostics(now_unix_seconds())
            .map(|queue| u64::try_from(queue.queued).unwrap_or(0))
            .unwrap_or(0),
        _ => 0,
    }
}

/// Swap the token of the session in memory and keep its keys and email. `false` when no session
/// is in memory.
fn replace_session_token_in_memory(acct: &AccountRuntime, token: &str) -> Result<bool, String> {
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    match guard.as_mut() {
        Some(session) => {
            session.token = token.to_string();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// What a re-sign-in resets: the revoked-session streak ends, and the session is present again.
fn reauth_settle_flags(state: &AppState, acct: &AccountRuntime, email: &str) {
    acct.auth_health.note_result(None);
    set_auth_present(state, true);
    set_auth_email(state, Some(email.to_string()));
}

/// What a successful authentication became on this Mac (R8).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignInSettlement {
    Fresh,
    Reauthenticated { vault_unlocked: bool },
    AccountMismatch { pending_changes: u64 },
}

/// R8: before anything is stored, decide whether this sign-in is fresh, the same account again, or
/// a different account. The same account gets only a new token (`reauth_in_place`). A different
/// account changes nothing here: the caller revokes the new session and returns the warning. Never
/// reads key material to decide: it asks only whether a vault key exists. `Fresh` requires that
/// nothing of a previous account remains (`LocalTraces`, Codex P1 on PR #113). Not on Windows,
/// where sign-in refuses while a session exists.
#[cfg(not(target_os = "windows"))]
async fn settle_sign_in(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: &str,
    profile: &account_dto::AccountProfile,
) -> Result<SignInSettlement, String> {
    let acct = state.active_account()?;
    let cfg = DesktopConfig::load().ok();
    let pending_changes = pending_changes_count();
    // R10: the recorded owner of the local data is the account on this computer. An unreadable
    // `state.db` counts as a trace (fail closed).
    let owner = local_data_owner();
    let recorded_owner = !matches!(owner, Ok(None));
    let owner = owner.ok().flatten();
    let keychain_email = keychain_account_email(acct.id.as_str());
    // Codex P1 on PR #113: every retained trace counts, including what a startup 401 keeps on
    // purpose (R9: the Keychain email and the vault key). A poisoned lock counts as a trace.
    let traces = reauth::LocalTraces {
        session_in_memory: acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true),
        auth_present: state.auth_present.lock().map(|guard| *guard).unwrap_or(true),
        keychain_token: keychain_session_present(acct.id.as_str()),
        keychain_email: keychain_email.is_some(),
        vault_key: keychain_vault_key_present(acct.id.as_str()),
        recorded_owner,
        cached_profile: acct.cached_profile.lock().map(|guard| guard.is_some()).unwrap_or(true),
        queued_or_staged: pending_changes > 0 || queued_or_staged_present(),
    };
    let user_id = owner.as_ref().and_then(|o| o.user_id.clone());
    let email = owner
        .and_then(|o| o.email)
        .or(keychain_email)
        .or_else(|| cfg.as_ref().and_then(|c| c.last_signed_in_email.clone()));
    let local = reauth::LocalAccount { user_id: user_id.as_deref(), email: email.as_deref(), traces: traces.any() };
    match reauth::sign_in_kind(&local, &profile.user_id, &profile.email) {
        reauth::SignInKind::Fresh => Ok(SignInSettlement::Fresh),
        reauth::SignInKind::DifferentAccount => Ok(SignInSettlement::AccountMismatch { pending_changes }),
        reauth::SignInKind::SameAccount => {
            let vault_unlocked = reauth_in_place(app, state, &acct, token, &profile.email, &profile.user_id).await?;
            Ok(SignInSettlement::Reauthenticated { vault_unlocked })
        }
    }
}

/// R8, same account: replace ONLY the session token. The queue, the cache, the Finder domain and
/// the vault key are never touched (`reauth_tests` pins it). Keys already in memory stay. Otherwise
/// the key this Mac still holds in the Keychain is loaded (a relaunch after a startup 401, Task 12).
/// Returns whether the vault is unlocked afterwards; `false` sends the person to the recovery phrase.
#[cfg(not(target_os = "windows"))]
async fn reauth_in_place(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    acct: &AccountRuntime,
    token: &str,
    email: &str,
    user_id: &str,
) -> Result<bool, String> {
    // R10: the same account (decided above) owns the local data. Recorded first: if this fails,
    // nothing has changed yet.
    record_local_data_owner(&account_binding::Identity::new(Some(user_id), Some(email)))?;
    persist_session_token_to_keychain(acct.id.as_str(), token, Some(email))?;
    let mut vault_unlocked = replace_session_token_in_memory(acct, token)?;
    if !vault_unlocked {
        match load_session_from_keychain(acct.id.as_str(), Some(email.to_string())) {
            Ok(Some(session)) => {
                install_unlocked_session(acct, session)?;
                vault_unlocked = true;
            }
            Ok(None) => {}
            Err(error) => tracing::info!(%error, "re-sign-in: no vault key on this Mac; the recovery phrase step follows"),
        }
    }
    reauth_settle_flags(state, acct, email);
    if vault_unlocked {
        let session = acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .as_ref()
            .map(|s| (s.token.clone(), s.master_key));
        if let Some((token, master_key)) = session {
            // A new engine with the new token; the queue in state.db is kept and drains.
            let started = start_engine_if_possible(app, state, token, master_key).await;
            keys_arrived(state);
            started?;
        }
    }
    tracing::info!(vault_unlocked, "signed in again in place (R8)");
    Ok(vault_unlocked)
}
```

Replace `struct LoginOutcome` with:

```rust
/// Result of a sign-in handed back to the frontend. `requires_2fa`: the password was right and a
/// TOTP code is still needed. R8 adds `reauthenticated` (the account on this Mac signed in again in
/// place; with `vault_unlocked` its keys are in memory, so no recovery phrase) and
/// `account_mismatch` (another account; nothing changed here, and the frontend shows the warning).
#[derive(serde::Serialize)]
struct LoginOutcome {
    requires_2fa: bool,
    reauthenticated: bool,
    vault_unlocked: bool,
    account_mismatch: Option<AccountMismatchDto>,
}

#[derive(serde::Serialize)]
struct AccountMismatchDto {
    pending_changes: u64,
}

impl LoginOutcome {
    fn signed_in() -> Self {
        Self { requires_2fa: false, reauthenticated: false, vault_unlocked: false, account_mismatch: None }
    }

    fn needs_2fa() -> Self {
        Self { requires_2fa: true, ..Self::signed_in() }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    fn reauthenticated(vault_unlocked: bool) -> Self {
        Self { reauthenticated: true, vault_unlocked, ..Self::signed_in() }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    fn account_mismatch(pending_changes: u64) -> Self {
        Self { account_mismatch: Some(AccountMismatchDto { pending_changes }), ..Self::signed_in() }
    }
}
```

Replace `Ok(LoginOutcome { requires_2fa: true })` with `Ok(LoginOutcome::needs_2fa())` and `Ok(LoginOutcome { requires_2fa: false })` with `Ok(LoginOutcome::signed_in())`.

- [ ] **Step 4b: The two probes behind `LocalTraces` (Spec issue 24), tests first**

`keychain.rs` test module:

```rust
    /// Codex P1 on PR #113: a retained vault key is seen without unlocking; a store that cannot
    /// hold secrets holds none; any other read error counts as present.
    #[test]
    fn holds_vault_key_sees_a_retained_key_and_fails_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(!holds_vault_key(&vault.store));
        vault.store_wrapped_master_key(SecretBytes::new_master_key([7u8; 32])).unwrap();
        assert!(holds_vault_key(&vault.store));

        struct Answers(fn() -> AuthStoreError);
        impl AuthSecretStore for Answers {
            fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> { Err((self.0)()) }
            fn load_session_token(&self) -> AuthResult<Option<SessionToken>> { Err((self.0)()) }
            fn delete_session_token(&self) -> AuthResult<()> { Err((self.0)()) }
            fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> { Err((self.0)()) }
            fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> { Err((self.0)()) }
            fn delete_wrapped_master_key(&self) -> AuthResult<()> { Err((self.0)()) }
            fn save_account_email(&self, _: &str) -> AuthResult<()> { Err((self.0)()) }
            fn load_account_email(&self) -> AuthResult<Option<String>> { Err((self.0)()) }
            fn delete_account_email(&self) -> AuthResult<()> { Err((self.0)()) }
        }
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::Unsupported("no keychain here"))));
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::NotFound)));
        assert!(holds_vault_key(&Answers(|| AuthStoreError::Backend("keychain locked".into()))), "fail closed");
    }
```

(If `AuthSecretStore` has more methods than the nine above, give each the same `Err((self.0)())` body.)

`state_db.rs` test module:

```rust
    #[test]
    fn queued_or_staged_counts_the_queue_and_the_staged_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.queued_or_staged_count().unwrap(), 0);
        db.track_staged_payload("/tmp/bb-p1-staged.bin", None, false).unwrap();
        assert_eq!(db.queued_or_staged_count().unwrap(), 1);
    }
```

RED: `cargo test --locked -p beebeeb-desktop --lib holds_vault_key` and `--lib queued_or_staged` into `$EVID/t11-red-probes.log`. Expected: compile errors (`cannot find function holds_vault_key`, `no method named queued_or_staged_count`). Then implement.

`keychain.rs`, module level next to `AuthVault`:

```rust
/// Codex P1 on PR #113: does this store still hold a vault key? It never unlocks, and the bytes
/// are dropped unread. A store that cannot hold secrets (`Unsupported`) or has none (`NotFound`)
/// holds none. Any other read error counts as present, so callers fail closed.
pub fn holds_vault_key<S: AuthSecretStore>(store: &S) -> bool {
    match store.load_wrapped_master_key() {
        Ok(key) => key.is_some(),
        Err(AuthStoreError::Unsupported(_) | AuthStoreError::NotFound) => false,
        Err(_) => true,
    }
}
```

`state_db.rs`, in `impl StateDb`:

```rust
    /// Codex P1 on PR #113: changes waiting to upload, or staged copies of them. A trace of an account.
    pub fn queued_or_staged_count(&self) -> Result<u64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let rows: i64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM operation_queue) + (SELECT COUNT(*) FROM staged_payloads)",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(rows).unwrap_or(0))
    }
```

GREEN: the same commands into `$EVID/t11-green-probes.log`. Expected: `ok. 1 passed` each.

- [ ] **Step 5: Wire the three sign-in methods**

`desktop_login`: add the parameter `app: tauri::AppHandle` first (Tauri injects it; the frontend call does not change), with `#[cfg_attr(target_os = "windows", allow(unused_variables))]` on it. Directly after `let profile = fetch_session_profile(&client, &base_url, &session_token).await?;` insert:

```rust
        // R8 (spec 2026-10-06): the account already on this Mac signs in again in place; another
        // account is an account switch. Decided before anything is stored.
        #[cfg(not(target_os = "windows"))]
        match settle_sign_in(app.clone(), &state, &session_token, &profile).await? {
            SignInSettlement::Fresh => {}
            SignInSettlement::Reauthenticated { vault_unlocked } => {
                if let Ok(acct) = state.active_account()
                    && let Ok(mut guard) = acct.cached_profile.lock()
                {
                    *guard = Some(profile);
                }
                return Ok(LoginOutcome::reauthenticated(vault_unlocked));
            }
            SignInSettlement::AccountMismatch { pending_changes } => {
                // Nothing on this Mac changed. The session just minted for the other account is
                // revoked; after the switch the person signs in again.
                let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                return Ok(LoginOutcome::account_mismatch(pending_changes));
            }
        }
```

A fresh sign-in records nothing here. The account binding (Task 10) records the owner at the first engine start, after any reset.

`desktop_login_2fa`: same parameter. Change the return type to `Result<LoginOutcome, String>`, apply the identical insertion after its `fetch_session_profile(…)` (its client/base_url/token variables have the same names), and change its final `Ok(())` to `Ok(LoginOutcome::signed_in())`.

`browser_login.rs` `run_handoff`: directly before `crate::apply_session(`, insert:

```rust
    // R8 (spec 2026-10-06): the same account signs in again in place; another account is an
    // account switch. The handoff carries no user id, so ask the server who this token is.
    #[cfg(not(target_os = "windows"))]
    {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .default_headers(crate::api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;
        let base_url = crate::runner::api_base_url();
        let profile = crate::fetch_session_profile(&client, &base_url, &creds.session_token).await?;
        match crate::settle_sign_in(app.clone(), state, &creds.session_token, &profile).await? {
            crate::SignInSettlement::Reauthenticated { vault_unlocked: true } => {
                emit(app, "done", serde_json::json!({ "email": email }));
                return Ok(());
            }
            // No keys on this Mac yet: apply_session below installs the handoff's key, which is this
            // same account's key. Nothing is purged either way.
            crate::SignInSettlement::Reauthenticated { vault_unlocked: false } | crate::SignInSettlement::Fresh => {
                // R10: with the profile cached, the account binding compares by user id.
                if let Ok(acct) = state.active_account()
                    && let Ok(mut guard) = acct.cached_profile.lock()
                {
                    *guard = Some(profile.clone());
                }
            }
            crate::SignInSettlement::AccountMismatch { pending_changes } => {
                let _ = crate::revoke_desktop_session(&client, &base_url, &creds.session_token).await;
                emit(app, "account_mismatch", serde_json::json!({ "pending_changes": pending_changes }));
                return Err("account_mismatch".to_string());
            }
        }
    }
```

(`Duration` is already imported in `browser_login.rs`. If not, use `std::time::Duration`.)

`clear_session_impl` needs no change in this task: the sign-out purge forgets the owner (Task 10).

`open_reauth_window`, next to `open_onboarding_window`:

```rust
/// R8: "Sign in again" on macOS opens sign-in in place. Nothing is cleared first, and the onboarding
/// window starts at the sign-in step whatever `sync_status` says (`mode=reauth`).
#[tauri::command]
async fn open_reauth_window(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Err("Only available on macOS and Linux.".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        const URL: &str = "index.html?window=onboarding&mode=reauth";
        if let Some(existing) = app.get_webview_window("onboarding") {
            existing
                .eval("window.location.search = '?window=onboarding&mode=reauth'")
                .map_err(|e| e.to_string())?;
            let _ = existing.show();
            let _ = existing.set_focus();
            return Ok(());
        }
        let window = tauri::WebviewWindowBuilder::new(&app, "onboarding", tauri::WebviewUrl::App(URL.into()))
            .title("Welcome to Beebeeb")
            .inner_size(860.0, 640.0)
            .min_inner_size(780.0, 560.0)
            .resizable(true)
            .center()
            .build()
            .map_err(|e| e.to_string())?;
        let _ = window.show();
        let _ = window.set_focus();
        Ok(())
    }
}
```

Register `open_reauth_window` in `generate_handler!` next to `open_onboarding_window`.

- [ ] **Step 6: GREEN**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib reauth > $EVID/t11-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t11-green.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib finder_setup_wiring_tests > $EVID/t11-census.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t11-census.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t11-check.log 2>&1; echo "rc=$?"
```

Expected: `reauth` filter → `ok. 14 passed` (6 in `reauth::tests` + 8 in `reauth_tests`; round 2 wrote 8, which miscounted its own module), the census module still passes, and `cargo check` gives `rc=0`. Windows compiles in CI's Windows job on the PR: the `#[cfg(target_os = "windows")]` arms cannot be checked from the Mac. Say so in the PR.

- [ ] **Step 7: Mutation checks (required for R8)**

1. Add `clear_keychain_session(acct.id.as_str())?;` as the first line of `reauth_in_place`. Expected: `a_re_sign_in_never_purges_removes_or_touches_keys` fails, naming `clear_keychain_session`. Remove it.
2. Add `persist_last_signed_in_email(None);` inside `desktop_login`'s `AccountMismatch` arm. Expected: `a_mismatch_changes_nothing_on_this_mac` fails (`persist_`). Remove it. Then add `record_local_data_owner(&account_binding::Identity::default())?;` there. Expected: the same test fails (`record_local_data_owner`). Remove it.
3. Delete the `#[cfg(not(target_os = "windows"))]` line above `desktop_login`'s `match settle_sign_in(`. Expected: `every_sign_in_method_is_checked_and_windows_is_unchanged` fails. Restore it.
4. In `replace_session_token_in_memory`, also write `session.master_key = [0u8; 32];`. Expected: `a_new_token_replaces_only_the_token_in_memory` fails (and the source test, on `master_key =`). Remove it.
5. In `reauth_in_place`, move the `record_local_data_owner(…)?;` line below `persist_session_token_to_keychain(…)?;`. Expected: `a_same_account_re_sign_in_records_the_owner_first` fails. Restore it.
6. In `settle_sign_in`, replace `vault_key: keychain_vault_key_present(acct.id.as_str()),` with `vault_key: false,`. Expected: `settle_sign_in_counts_every_retained_trace` fails (`keychain_vault_key_present(`). Restore it.

Paste each failure into Notes.

- [ ] **Step 8: Commit**

```bash
cd $WT && git add src-tauri/src/reauth.rs
git commit -m "auth (R8): re-sign-in in place: same account swaps only the token; another account is a switch; Windows unchanged

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/reauth.rs src-tauri/src/lib.rs src-tauri/src/browser_login.rs
git show --stat HEAD
```

- [ ] **Step 9: Mandatory security review (lead), before Lane R's PR merges**

After Task 12 lands, the lead runs the `crypto-security-reviewer` agent on the combined diff of Tasks 10, 11 and 12 (`git diff <Task 9 commit>..HEAD -- src-tauri/src/account_binding.rs src-tauri/src/state_db.rs src-tauri/src/reauth.rs src-tauri/src/lib.rs src-tauri/src/browser_login.rs src-tauri/src/keychain.rs src-tauri/src/account_dto.rs`). The prompt also gives the reviewer the path of the private task, for the R10 background. It names these questions:

1. Can a different account ever be treated as `SameAccount`, by R8 or by the binding? Look at the email fallback and the upgrade adoption of Spec issue 19, and at an owner record that is missing, email-only or stale.
2. Can local data ever be used under another account's session? Check every engine start (`spawn_bound_engine`), the upgrade adoption, R8's owner recording, and the Windows refusal.
3. Is key material ever logged, persisted twice, or left un-zeroized on the new paths?
4. Is the revoked token's replacement atomic enough? What happens if the Keychain write succeeds and the in-memory swap fails?
5. Spec issue 18, ruled R9 ("Keep the key, review decides"): keys are kept on the Mac after a remote revocation. Is that acceptable, and what would the reviewer require? A "no" goes back to Guus.
6. The mismatch path revokes the new session. Can it leave a dangling server session?
7. R10: is a failed reset or a failed sign-out purge always fatal before any engine starts? Is `clear_account_data` complete (`every_table_is_classified_for_the_account_binding`)?
8. R10 on Windows refuses instead of resetting (Spec issue 17, ruled R11: refuse here; the escape hatch is private task 1837). Is the resulting state safe?
9. Codex P1 on PR #113 (Spec issue 24): can any retained trace be missed, so that another account gets `Fresh`? The traces are the recorded owner, the Keychain email, the vault key, the cached profile, and queued or staged data. Can the previous account's vault key ever sit next to another account's token, or be loaded for it?

The findings go verbatim into the task Notes under "Security review (R8, R9, R10)", and the R10 findings also into the private task's Notes. A HIGH finding blocks the merge until it is fixed (back to the owning task) or Guus rules on it. Without this step, Lane R's PR does not merge.

---

## Task 12: A startup 401 keeps the keys and Finder; the startup probe caches the account profile (R8, R9, backend)

**Lane R.** Today a startup 401 (`discard_unusable_startup_session`) deletes the session token **and the vault key and email**. Under R8 the same account must be able to sign in again with "keys … all stay", so on macOS and Linux only the revoked token is deleted. The Finder domain already stays: `finder_signed_out_by_choice` is not set by this path (Task 6, core `wanted`). The startup probe also caches the account profile while the session still works, so the account binding (Task 10) compares by user id instead of email. The token-only discard is not for Windows: Windows keeps today's discard. Because the key and the email now survive a startup 401, "already signed out" no longer means "nothing left". The sign-out that an account switch runs must therefore verify that no vault key and no email remain, or stop (Codex P1 on PR #113, Spec issue 24).

**This task changes key retention after a remote revocation (Spec issue 18, ruled R9: "Keep the key, review decides").** It merges only with the security review's explicit OK on that point (Task 11 step 9, question 5). If the review disagrees, the lead takes the question back to Guus. The lane neither drops nor merges the change on its own. Without the change, a same-account re-sign-in still keeps Finder and the queue, but asks for the recovery phrase after a relaunch.

**Files:**
- Modify: `src-tauri/src/keychain.rs` (`AuthVault::clear_session_token` + a test using the existing in-module `MemoryStore`, line ~918)
- Modify: `src-tauri/src/lib.rs`: `clear_keychain_session_token` (non-Windows), `discard_unusable_startup_session` (cfg split), `StartupSessionCheck::Authorized { profile }`, `probe_startup_session` (parses the profile), `restore_session_on_startup` (caches it), `ensure_keychain_holds_no_account` and its two calls in `clear_session_impl` (a switch leaves no vault key behind), the `AuthMeMockServer` test helper (a body), and tests
- Modify: `src-tauri/src/account_dto.rs` (`AccountProfile` derives `PartialEq, Eq`, so `StartupSessionCheck` keeps its `Eq`)

**Interfaces:**
- Consumes: `account_dto::AccountProfile`. The account binding (Task 10) reads `acct.cached_profile`, which this task fills at startup.
- Produces: `AuthVault::clear_session_token(&mut self) -> AuthResult<()>`; `StartupSessionCheck::Authorized { profile: Option<account_dto::AccountProfile> }`.

- [ ] **Step 1: Tests first**

`keychain.rs` test module (next to the existing `MemoryStore` tests):

```rust
    #[test]
    fn clearing_the_session_token_keeps_the_key_and_the_email() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault.store_wrapped_master_key(SecretBytes::new_master_key([9u8; 32])).unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(vault.session_token().unwrap().is_none(), "the revoked token is gone");
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("sam@beebeeb.io"));
        assert!(vault.store.load_wrapped_master_key().unwrap().is_some(), "R8: the vault key stays");
    }

    /// Codex P1 on PR #113: after the token-only clear of a startup 401, the full clear of
    /// "Sign out and switch" leaves no vault key and no email for the next account.
    #[test]
    fn a_full_clear_after_a_token_only_clear_leaves_no_vault_key() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok-a").unwrap()).unwrap();
        vault.store_wrapped_master_key(SecretBytes::new_master_key([7u8; 32])).unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(holds_vault_key(&vault.store), "R9: the revoked token goes, the key stays");
        vault.clear_session().unwrap();
        assert!(!holds_vault_key(&vault.store), "after the switch the old key is gone");
        assert_eq!(vault.account_email().unwrap(), None);
    }
```

If `MemoryStore` does not implement `Default` or `save_account_email`, build it the way the neighbouring tests do. The assertions stay the same.

`lib.rs`, in `startup_session_tests`:

```rust
    #[test]
    fn a_startup_401_drops_only_the_token_on_macos_and_linux_and_everything_on_windows() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let non_windows = production
            .find("#[cfg(not(target_os = \"windows\"))]\nfn discard_unusable_startup_session(")
            .expect("a non-Windows discard");
        let body = &production[non_windows..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("clear_keychain_session_token("), "R8: only the token goes");
        assert!(!body.contains("clear_keychain_session("), "the keys and email stay");
        let windows = production
            .find("#[cfg(target_os = \"windows\")]\nfn discard_unusable_startup_session(")
            .expect("Windows keeps its discard");
        let body = &production[windows..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("clear_keychain_session(acct.id.as_str())"), "Windows is unchanged");
    }

    #[tokio::test]
    async fn an_authorized_probe_returns_the_account_profile() {
        let server = AuthMeMockServer::start_with_body("200 OK", r#"{"user_id":"u-1","email":"sam@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#);
        match probe_startup_session(&server.base_url, "stored-token").await {
            StartupSessionCheck::Authorized { profile: Some(profile) } => assert_eq!(profile.user_id, "u-1"),
            other => panic!("expected an authorized probe with a profile, got {other:?}"),
        }
        let unparseable = AuthMeMockServer::start_with_body("200 OK", "{}");
        assert_eq!(
            probe_startup_session(&unparseable.base_url, "stored-token").await,
            StartupSessionCheck::Authorized { profile: None },
            "an unreadable body still means authorized; only the profile is missing"
        );
    }

    #[test]
    fn the_startup_restore_caches_the_probed_profile() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let restore = &source[source.find("async fn restore_session_on_startup(").unwrap()..];
        let restore = &restore[..restore.find("\n}\n").unwrap()];
        let arm = &restore[restore.find("StartupSessionCheck::Authorized { profile }").expect("the authorized arm")..];
        assert!(arm[..arm.find("StartupSessionCheck::Inconclusive").unwrap()].contains("cached_profile"));
    }

    /// Codex P1 on PR #113: "already signed out" can still hold the vault key and the email (R9). Both
    /// sign-out branches verify that nothing of the account is left before they report success.
    #[test]
    fn a_switch_after_a_revoked_token_leaves_no_vault_key_behind() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let clear = &production[production.find("async fn clear_session_impl(").unwrap()..];
        let clear = &clear[..clear.find("\n}\n").unwrap()];
        let already = &clear[clear.find("if already_signed_out {\n        // Already signed out").expect("the already-signed-out branch")..];
        let already = &already[..already.find("return Ok(SignOutOutcome::NotSignedIn);").unwrap()];
        assert!(already.contains("ensure_keychain_holds_no_account(acct.id.as_str())?;"), "the already-signed-out branch verifies");
        let tail = &clear[clear.rfind("clear_keychain_session(acct.id.as_str())?;").unwrap()..];
        assert!(tail.contains("ensure_keychain_holds_no_account(acct.id.as_str())?;"), "the signed-in branch verifies");
    }
```

Extend the test helper: `impl AuthMeMockServer { fn start_with_body(status_line: &'static str, body: &'static str) -> Self }`, the same as `start` but replying with `body` (and `Content-Type: application/json`). `start(status_line)` becomes `start_with_body(status_line, "{}")`. Update the existing probe tests' `StartupSessionCheck::Authorized` assertions to `StartupSessionCheck::Authorized { profile: None }`.

RED: `cargo test --locked -p beebeeb-desktop --lib startup_session_tests` and `--lib keychain` into `$EVID/t12-red.log`. Expected: compile errors (`no method clear_session_token`, `start_with_body`, variant shape).

- [ ] **Step 2: Implement**

`keychain.rs`, in `impl<S: AuthSecretStore> AuthVault<S>` after `clear_session`:

```rust
    /// Remove only the session token: R8, a token the server revoked, found at startup. The wrapped
    /// vault key and the account email stay; `clear_session` remains the full sign-out.
    pub fn clear_session_token(&mut self) -> AuthResult<()> {
        self.store.delete_session_token()
    }
```

`lib.rs`, next to `clear_keychain_session`:

```rust
/// R8: a startup 401 drops only the revoked token, in the id-keyed and the legacy store. The vault
/// key and the email stay, so the same account signs in again without its recovery phrase.
#[cfg(not(target_os = "windows"))]
fn clear_keychain_session_token(account_id: &str) -> Result<(), String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .clear_session_token()
        .map_err(|e| keychain_error("clear Keychain session token", e))?;
    AuthVault::new(keychain::legacy_platform_keychain_store())
        .clear_session_token()
        .map_err(|e| keychain_error("clear legacy Keychain session token", e))
}
```

And the verification an account switch needs (Spec issue 24):

```rust
/// Codex P1 on PR #113: a startup 401 keeps the vault key and the email (R9), so a sign-out (and
/// with it "Sign out and switch") must leave neither for the next account. A key that cannot be
/// read counts as still there (fail closed); a store that cannot hold secrets has nothing to leave.
#[cfg(not(target_os = "windows"))]
fn ensure_keychain_holds_no_account(account_id: &str) -> Result<(), String> {
    if keychain_vault_key_present(account_id) || keychain_account_email(account_id).is_some() {
        return Err(SIGN_OUT_KEYCHAIN_NOT_CLEARED.to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
const SIGN_OUT_KEYCHAIN_NOT_CLEARED: &str =
    "Sign-out paused: Beebeeb couldn’t remove the previous account’s keys from this computer. Try again; if it keeps happening, restart Beebeeb.";
```

In `clear_session_impl`, call it in both branches:
- In the already-signed-out branch, directly after its `if let Err(error) = clear_keychain_session(acct.id.as_str()) { … }` block.
- In the signed-in branch, directly after `clear_keychain_session(acct.id.as_str())?;`.

```rust
    #[cfg(not(target_os = "windows"))]
    ensure_keychain_holds_no_account(acct.id.as_str())?;
```

In the already-signed-out branch's comment, replace "the trio should already be gone" with "after a startup 401 (R9) the vault key and the email can still be here".

Split `discard_unusable_startup_session`. The existing function gets `#[cfg(target_os = "windows")]` and stays byte-for-byte as it is. Add:

```rust
/// R8 (spec 2026-10-06): the server rejected the stored token at startup. Drop only that token,
/// keep the vault key and the email, and boot signed out. The next sign-in as the same account
/// swaps in a new token and needs no recovery phrase (`reauth_in_place`). Beebeeb stays in Finder
/// (this is not a sign-out by choice). Another account goes through the switch: full sign-out first.
#[cfg(not(target_os = "windows"))]
fn discard_unusable_startup_session(state: &AppState, acct: &crate::account::AccountRuntime, email: Option<String>) {
    tracing::info!(
        account_id = acct.id.as_str(),
        "stored session token was rejected by the server (HTTP 401) at startup; dropping the token, keeping the keys (R8)"
    );
    persist_last_signed_in_email(email.or_else(|| signout_email_to_preserve(acct)));
    if let Err(error) = clear_keychain_session_token(acct.id.as_str()) {
        tracing::warn!(%error, "could not clear the server-rejected session token at startup (best-effort)");
    }
    set_auth_present(state, false);
    set_auth_email(state, None);
}
```

`account_dto.rs`: add `PartialEq, Eq` to `AccountProfile`'s derive list (every field is a `String`, `bool` or `Option` of those).

`StartupSessionCheck::Authorized` becomes `Authorized { profile: Option<account_dto::AccountProfile> }`. In `probe_startup_session`, the success arm:

```rust
        status if status.is_success() => {
            let profile = tokio::time::timeout(std::time::Duration::from_secs(5), response.json::<account_dto::AccountProfile>())
                .await
                .ok()
                .and_then(Result::ok);
            StartupSessionCheck::Authorized { profile }
        }
```

In `restore_session_on_startup`, the arm `StartupSessionCheck::Authorized => {}` becomes:

```rust
        StartupSessionCheck::Authorized { profile } => {
            // R10: the session's account, so the account binding compares by user id (Task 10).
            if let Some(profile) = profile
                && let Ok(mut guard) = acct.cached_profile.lock()
            {
                *guard = Some(profile);
            }
        }
```

The existing `discard_unusable_startup_session_boots_signed_out` test keeps passing on Linux: its flag assertions are unchanged.

- [ ] **Step 3: GREEN**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib startup_session_tests > $EVID/t12-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t12-green.log
$LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib keychain > $EVID/t12-keychain.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t12-keychain.log
$LOCK cargo-build -- cargo test --locked --test keychain > $EVID/t12-keychain-it.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/t12-keychain-it.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t12-check.log 2>&1; echo "rc=$?"
```

Expected: `0 failed` everywhere, with the counts recorded in Notes. The `keychain` integration binary is unchanged (it already tests the trio).

- [ ] **Step 4: Mutation check**

In the non-Windows discard, replace `clear_keychain_session_token(` with `clear_keychain_session(`. Expected: `a_startup_401_drops_only_the_token_…` fails. Restore it. In `clear_session_token`, also call `self.store.delete_wrapped_master_key()?;`. Expected: `clearing_the_session_token_keeps_the_key_and_the_email` fails. Restore it. Delete the `ensure_keychain_holds_no_account(acct.id.as_str())?;` call in the already-signed-out branch. Expected: `a_switch_after_a_revoked_token_leaves_no_vault_key_behind` fails (`the already-signed-out branch verifies`). Restore it. The RED for that test is the same failure, seen before the calls are added: paste it into Notes.

- [ ] **Step 5: Commit**

```bash
cd $WT && git commit -m "auth (R8, R9): a startup 401 drops only the revoked token on macOS/Linux; the startup probe caches the account profile

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src-tauri/src/keychain.rs src-tauri/src/lib.rs src-tauri/src/account_dto.rs
git show --stat HEAD
```

Then the lead runs Task 11 step 9 (the security review) on Tasks 10, 11 and 12, and only then Task 9 step 6 (open Lane R's PR).

---

## Task 13: Design artefacts change first (spec §12)

**Lane T, first commit on `feat/$TASK-finder-reconciler-ui`.** No frontend commit may come before this one.

**Files:**
- Modify: `design/hifi/macos-menubar-popover.html`
- Modify: `design/hifi/macos-settings-dialogs.html` (lines 210, 216, 327; a new dialog 4, the R8 account-switch warning)
- Modify: `docs/specs/2026-09-30-macos-menubar-popover.md` (one amendment line)

**How the 1.8 MB artefact is edited.** It is 93 lines. Line 52 is 1.24 MB and holds the ten popover artboards as `<figure class="shot"><img src="data:image/png;base64,…" alt="Popover, … state" …><figcaption>…</figcaption></figure>`. Measured at `c351f98`: 5 lines over 2 kB, and the `alt` texts include `Popover, Finder not added state` and `Popover, Finder add failed state`. The figures are PNG renders, and their source (`design/hifi/macos-menubar-popover.jsx`, named on line 91) is **not in the repo**, so the drawing cannot be regenerated. The edit therefore (a) deletes the "Finder not added" `<figure>`, (b) replaces the "Finder add failed" `<figure>` with an HTML-drawn artboard built from the page's own tokens, so the copy is text that a test can pin against `finderSetupCopy.ts`, and (c) adds the artboard's CSS to the page's `<style>` and a dated amendment note. A Python script does it with exact anchors, and every anchor count is asserted, because an editor cannot handle a 1.2 MB line safely.

- [ ] **Step 1: Write the edit script in a scratch dir and run it**

```bash
SCRATCH=$(mktemp -d)
cat > "$SCRATCH/edit_popover_artefact.py" <<'PY'
"""Spec 2026-10-06 §12: remove the "Finder not added" artboard; redraw "Finder add failed" with the
reason-keyed copy of §6.2 (typographic apostrophes, plan "Spec issues" 15)."""
import pathlib, sys

path = pathlib.Path(sys.argv[1])
html = path.read_text(encoding="utf-8")

def figure_span(alt):
    marker = f'alt="{alt}"'
    assert html.count(marker) == 1, f"{marker}: {html.count(marker)}"
    at = html.index(marker)
    start = html.rindex('<figure class="shot">', 0, at)
    end = html.index("</figure>", at) + len("</figure>")
    return start, end

ROWS = [
    ("adding", "status", "Adding Beebeeb to Finder…", None),
    ("extension_loading", "alert", "macOS hasn’t finished loading Beebeeb’s Finder extension.", "Try again"),
    ("user_disabled", "status", "Beebeeb is turned off in System Settings.", "Open System Settings"),
    ("not_in_applications", "alert", "Beebeeb is running from the disk image. Move it to Applications, then open it again.", "Show in Finder"),
    ("folder_taken", "alert", "An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.", "Copy details"),
    ("signing", "alert", "This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.", "Copy details"),
    ("timeout", "alert", "macOS didn’t finish adding Beebeeb to Finder.", "Try again"),
    ("unknown", "alert", "Beebeeb couldn’t be added to Finder.", "Try again"),
]

def row(tag, tone, sentence, action):
    button = f'<span class="board-btn">{action}</span>' if action else ""
    return f'<div class="board-row board-{tone}"><span class="board-tag">{tag}</span><p>{sentence}</p>{button}</div>'

FIGURE = (
    '<figure class="shot" id="finder-add-failed"><div class="board">'
    '<div class="board-head">Beebeeb in Finder</div>'
    + "".join(row(*r) for r in ROWS)
    + '</div><figcaption><strong>Finder add failed</strong><span>Shown only after the silent retries: '
    'one sentence and one action per reason (spec 2026-10-06 §6.2). “Try again” appears only here. '
    'Replaces the single timeout drawing.</span></figcaption></figure>'
)

STYLE = (
    ".board{background:var(--surface);border:1px solid var(--line);border-radius:10px;padding:14px;display:flex;flex-direction:column;gap:8px;font-size:13px;line-height:1.4}\n"
    ".board-head{font-weight:600;font-size:14px}\n"
    ".board-row{display:grid;grid-template-columns:1fr auto;gap:4px 10px;align-items:center;padding:8px 10px;border-radius:8px;background:var(--bg);border:1px solid var(--line)}\n"
    ".board-alert{border-left:3px solid var(--ink)}\n"
    ".board-tag{grid-column:1/-1;font:500 11px var(--mono);color:var(--muted)}\n"
    ".board-row p{margin:0;min-width:0}\n"
    ".board-btn{font-size:12px;padding:3px 10px;border:1px solid var(--line);border-radius:6px;white-space:nowrap}\n"
)

AMENDMENT = (
    '<p class="sub"><strong>Amended 6 Oct 2026</strong> (ruling R5, '
    '<code>docs/specs/2026-10-06-macos-finder-setup-reconciler.md</code>): the “Finder not added” state '
    'and its “Add to Finder” button are gone. Beebeeb adds itself after sign-in. “Finder add failed” '
    'shows one sentence and one action per reason.</p>'
)

s, e = figure_span("Popover, Finder not added state")
html = html[:s] + html[e:]
s, e = figure_span("Popover, Finder add failed state")
html = html[:s] + FIGURE + html[e:]

anchor = '</style>\n<div class="wrap">'
assert html.count(anchor) == 1, anchor
html = html.replace(anchor, STYLE + anchor)

anchor = '<p class="sub">Fixed at 372×488 pt'
assert html.count(anchor) == 1, anchor
end = html.index("</p>", html.index(anchor)) + len("</p>")
html = html[:end] + AMENDMENT + html[end:]

path.write_text(html, encoding="utf-8")
print("ok")
PY
cd $WT && python3 "$SCRATCH/edit_popover_artefact.py" design/hifi/macos-menubar-popover.html
```

Expected output: `ok`.

- [ ] **Step 2: Verify the artefact by counts**

```bash
cd $WT && f=design/hifi/macos-menubar-popover.html
grep -c 'Finder not added' $f                      # expect 0
grep -o 'alt="Popover, [^"]*"' $f | wc -l          # expect 8 (was 10)
grep -c 'id="finder-add-failed"' $f                # expect 1
grep -o 'class="board-tag"' $f | wc -l             # expect 8 (adding + 7 reasons)
grep -o 'class="board-btn"' $f | wc -l             # expect 7
wc -l $f                                           # expect 93: no line was added or lost
```

Write all six numbers into Notes.

- [ ] **Step 3: Render it in light and dark and look**

Open the file in a browser and screenshot the `#finder-add-failed` figure in both schemes. With Playwright available (`PLAYWRIGHT_MODULE` as in `tests/render-mac-settings.mjs`):

```bash
cat > "$SCRATCH/shot.mjs" <<'JS'
import { pathToFileURL } from 'node:url'
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright')
const [file, out, selector = '#finder-add-failed'] = process.argv.slice(2)
const browser = await chromium.launch()
for (const colorScheme of ['light', 'dark']) {
  const page = await browser.newPage({ colorScheme, viewport: { width: 1200, height: 900 } })
  await page.goto(pathToFileURL(file).href)
  await page.locator(selector).screenshot({ path: `${out}-${colorScheme}.png` })
}
await browser.close()
JS
node "$SCRATCH/shot.mjs" "$WT/design/hifi/macos-menubar-popover.html" "$EVID/design-finder-add-failed"
```

Without Playwright, the lead takes the same two screenshots with the Chrome DevTools MCP (open the `file://` URL; `emulate` `prefers-color-scheme: dark`). Expected: eight rows, each with its tag, one sentence and at most one button, readable in both schemes, no amber, and no horizontal overflow at 1200 px. Then check the artboard at phone width (375 px): rows wrap and the page does not scroll sideways. Evidence: `$EVID/design-finder-add-failed-{light,dark}.png`.

- [ ] **Step 4: The Settings-dialog artefact and the popover spec**

In `design/hifi/macos-settings-dialogs.html`, make three exact edits:

1. Line 216: replace `<p class="sheet-copy">Beebeeb removes its Finder location and turns off Open Beebeeb at login. Files waiting to upload are kept. You can add it back afterwards.</p>` with `<p class="sheet-copy">Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.</p>`.
2. Line 327: in the table cell, replace `body “Beebeeb removes its Finder location and turns off Open Beebeeb at login. Files waiting to upload are kept. You can add it back afterwards.”` with `body “Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.” (amended 6 Oct 2026, ruling R5)`.
3. Line 210: keep the existing sentence and append, before `</p></div>`: ` <strong>Amended 6 Oct 2026 (ruling R5):</strong> Repair now adds Beebeeb back to Finder by itself, so it is still a caution, not a destruction.`

In `docs/specs/2026-09-30-macos-menubar-popover.md`, add this directly after the `**Implementation plan:** …` line, leaving the original `States:` bullet as it is:

```markdown
**Amended 2026-10-06 (ruling R5, [spec A](2026-10-06-macos-finder-setup-reconciler.md) §12):** the "finder not added" state and its "Add to Finder" action are removed: Beebeeb adds itself to Finder after sign-in. "Finder add failed" shows one sentence and one action per reason (spec A §6.2), and "Try again" survives only inside a failure. The `States:` bullet below keeps its original wording for the record.
```

Verify: `grep -c "add it back afterwards" design/hifi/macos-settings-dialogs.html` gives exactly 1 (the quoted old copy in line 210's original sentence, kept visible), and `grep -c "adds itself back to Finder" design/hifi/macos-settings-dialogs.html` gives 2.

- [ ] **Step 4b: Draw the R8 account-switch warning (design first, for Task 18)**

Ruling R8 needs one new dialog: the warning before an account switch. Add it to `design/hifi/macos-settings-dialogs.html` as dialog 4, its own section directly after section 3 (`s-folders`). It is built from the Repair dialog's markup, and its confirm button is the shipped destructive `ms-btn--danger` (the Sign out dialog's, PR #104, already styled at line 108), so it reads as the same system.

```bash
python3 - "$WT/design/hifi/macos-settings-dialogs.html" <<'PY'
import pathlib, sys
p = pathlib.Path(sys.argv[1]); html = p.read_text(encoding="utf-8")
start = html.index('<section aria-labelledby="s-folders">')
end = html.index("</section>", start) + len("</section>")
def dialog(body):
    return ('<div class="stage"><div class="modal" role="dialog" aria-label="Switch to a different account?">'
            '<div class="modal-head"><div class="modal-title">Switch to a different account?</div>'
            '<button type="button" class="modal-x" aria-label="Close">&times;</button></div>'
            f'<div class="modal-body"><p class="sheet-copy">{body}</p></div>'
            '<div class="modal-foot"><button type="button" class="ms-btn">Cancel</button>'
            '<button type="button" class="ms-btn ms-btn--danger">Sign out and switch</button></div></div></div>')
SECTION = (
    '\n\n<section aria-labelledby="s-switch">\n'
    '  <div><h2 id="s-switch">4 · Switch to a different account — confirmation</h2>\n'
    '  <p class="sub"><strong>Added 6 Oct 2026</strong> (ruling R8, <code>docs/specs/2026-10-06-macos-finder-setup-reconciler.md</code>). '
    'Shown when a sign-in is a different account from the one on this Mac. The same account never sees it: only its session token '
    'is replaced, and Finder, keys, cache and pending edits stay. Cancel changes nothing. The confirm button is destructive: it signs '
    'the other account out of this Mac and removes changes that have not uploaded.</p></div>\n'
    '  <div class="grid2">\n'
    '    <figure class="shot">' + dialog('This Mac is signed in to another Beebeeb account, and 3 changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.')
    + '<figcaption><b>Changes waiting</b><span>The number is the upload queue on this Mac. One change reads “1 change … hasn’t uploaded yet … removes it.”</span></figcaption></figure>\n'
    '    <figure class="shot">' + dialog('This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.')
    + '<figcaption><b>Nothing waiting</b><span>No count, and no loss to name.</span></figcaption></figure>\n'
    '  </div>\n'
    '</section>'
)
assert html.count('<section aria-labelledby="s-switch">') == 0
html = html[:end] + SECTION + html[end:]
p.write_text(html, encoding="utf-8")
print("ok")
PY
grep -o 'Switch to a different account?' "$WT/design/hifi/macos-settings-dialogs.html" | wc -l   # expect 4
grep -o 'ms-btn--danger' "$WT/design/hifi/macos-settings-dialogs.html" | wc -l                   # expect 8 (6 at c351f98, +2)
grep -c 'id="s-switch"' "$WT/design/hifi/macos-settings-dialogs.html"                            # expect 1
```

Write the three numbers into Notes. Screenshot the new section in light and dark with step 3's script, passing the section as the selector:

```bash
node "$SCRATCH/shot.mjs" "$WT/design/hifi/macos-settings-dialogs.html" "$EVID/design-account-switch" 'section[aria-labelledby="s-switch"]'
```

Expected: both dialogs readable in both schemes, the confirm button red with legible text, and no horizontal overflow at 375 px. The copy is new. Guus reviews it in this PR (Spec issue 21). Task 18's copy test holds `src/accountSwitchCopy.ts` to these exact strings.

- [ ] **Step 5: Commit (design first)**

```bash
cd $WT && git commit -m "design: popover Finder states per spec A (R5): no 'Finder not added'; reason-keyed 'Finder add failed'; Repair copy; R8 account-switch warning

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- design/hifi/macos-menubar-popover.html design/hifi/macos-settings-dialogs.html docs/specs/2026-09-30-macos-menubar-popover.md
git show --stat HEAD
rm -rf "$SCRATCH"
```

`git show --stat HEAD` lists exactly 3 files.

---

## Task 14: The frontend contract: `finderSetup.ts` and `finderSetupCopy.ts`

**Lane T.**

**Files:**
- Create: `src/finderSetup.ts`
- Create: `src/finderSetupCopy.ts`
- Create: `tests/finderSetupCopy.test.ts`

**Interfaces:**
- Consumes: `command`, `CommandResult` (`src/desktopApi.ts`); `listen` (`@tauri-apps/api/event`); the Rust JSON shape of `FinderSetupView` (Task 7).
- Produces (used by Tasks 15–17):
  - `finderSetup.ts`: `FINDER_FAILURE_REASONS` (readonly tuple), `type FinderFailureReason`, `type FinderSetupState = 'ready' | 'missing' | 'adding' | 'failed' | 'user_disabled'`, `type LaunchLocation`, `interface FinderFailureRecord`, `interface FinderSetupView`, `FINDER_SETUP_CHANGED_EVENT`, `FINDER_ACTION_COMMAND: Record<FinderSetupAction, string>`, `loadFinderSetup()`, `subscribeFinderSetup(onView) => unsubscribe`, `interface FinderActionDeps { writeClipboard?: (text: string) => Promise<void> }`, `copyFinderSetupDetails(deps?)`, `runFinderSetupAction(action, deps?)`.
  - `finderSetupCopy.ts`: `type FinderSetupAction`, `FINDER_SETUP_TITLE`, `FINDER_ADDING_LINE`, `FINDER_READY_LINE`, `FINDER_REASON_COPY`, `FINDER_ACTION_LABEL`, `FINDER_ACTION_FAILED`, `FINDER_STATUS_PILL`, `type FinderSetupPresentation`, `finderSetupPresentation(view | null)`.

- [ ] **Step 1: Write the test**

`tests/finderSetupCopy.test.ts`:

```ts
/**
 * Spec 2026-10-06 §6.2 and §10: every Finder-setup string lives in src/finderSetupCopy.ts, keyed
 * by reason; each reason is one sentence and one action; the design artefact draws exactly this
 * copy (§12). Strings use the house typographic apostrophe (plan "Spec issues" 15).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { copyFinderSetupDetails, FINDER_FAILURE_REASONS, type FinderFailureReason, type FinderSetupView } from '../src/finderSetup'
import {
  FINDER_ACTION_FAILED,
  FINDER_ACTION_LABEL,
  FINDER_ADDING_LINE,
  FINDER_READY_LINE,
  FINDER_REASON_COPY,
  FINDER_SETUP_TITLE,
  FINDER_STATUS_PILL,
  finderSetupPresentation,
} from '../src/finderSetupCopy'

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'missing',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 1,
  last_failure: null,
  ...over,
})

const TABLE: Record<FinderFailureReason, [string, string]> = {
  extension_loading: ['macOS hasn’t finished loading Beebeeb’s Finder extension.', 'Try again'],
  user_disabled: ['Beebeeb is turned off in System Settings.', 'Open System Settings'],
  not_in_applications: ['Beebeeb is running from the disk image. Move it to Applications, then open it again.', 'Show in Finder'],
  folder_taken: ['An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.', 'Copy details'],
  signing: ['This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.', 'Copy details'],
  timeout: ['macOS didn’t finish adding Beebeeb to Finder.', 'Try again'],
  unknown: ['Beebeeb couldn’t be added to Finder.', 'Try again'],
}

describe('finderSetupCopy (spec §6.2)', () => {
  test('every reason has exactly the sentence and the action of the table', () => {
    expect(Object.keys(FINDER_REASON_COPY).sort()).toEqual([...FINDER_FAILURE_REASONS].sort())
    for (const reason of FINDER_FAILURE_REASONS) {
      const copy = FINDER_REASON_COPY[reason]
      expect([copy.sentence, FINDER_ACTION_LABEL[copy.action]]).toEqual(TABLE[reason])
    }
  })

  test('the fixed lines', () => {
    expect(FINDER_ADDING_LINE).toBe('Adding Beebeeb to Finder…')
    expect(FINDER_READY_LINE).toBe('Your vault appears under Locations in Finder.')
    expect(FINDER_SETUP_TITLE).toBe('Beebeeb in Finder')
  })

  test('each reason presents one notice with its action; user_disabled is a neutral status', () => {
    for (const reason of FINDER_FAILURE_REASONS) {
      const setup = reason === 'user_disabled' ? 'user_disabled' : 'failed'
      expect(finderSetupPresentation(view({ setup, reason }))).toEqual({
        kind: 'notice',
        tone: reason === 'user_disabled' ? 'status' : 'alert',
        reason,
        sentence: TABLE[reason][0],
        action: FINDER_REASON_COPY[reason].action,
        actionLabel: TABLE[reason][1],
      })
    }
  })

  test('the states that are not failures', () => {
    expect(finderSetupPresentation(null)).toEqual({ kind: 'quiet', line: '' })
    expect(finderSetupPresentation(view({ setup: 'missing' }))).toEqual({ kind: 'quiet', line: '' })
    expect(finderSetupPresentation(view({ setup: 'adding' }))).toEqual({ kind: 'adding', line: FINDER_ADDING_LINE })
    expect(finderSetupPresentation(view({ setup: 'ready' }))).toEqual({ kind: 'ready', line: FINDER_READY_LINE })
    expect(finderSetupPresentation(view({ setup: 'failed' }))).toMatchObject({ kind: 'notice', reason: 'unknown' })
  })

  test('Try again appears only inside a failure', () => {
    for (const v of [view({ setup: 'ready' }), view({ setup: 'adding' }), view({ setup: 'missing' }), view({ setup: 'user_disabled', reason: 'user_disabled' })]) {
      const p = finderSetupPresentation(v)
      expect(p.kind === 'notice' ? p.actionLabel : null).not.toBe('Try again')
    }
  })

  test('from the disk image the reason shows before sign-in (D7)', () => {
    expect(finderSetupPresentation(view({ setup: 'missing', reason: 'not_in_applications', launch_location: 'disk_image' }))).toMatchObject({
      kind: 'notice',
      action: 'show_in_finder',
    })
  })

  test('no Finder-setup copy offers to add or install (R5)', () => {
    const all = [
      FINDER_ADDING_LINE,
      FINDER_READY_LINE,
      FINDER_SETUP_TITLE,
      ...Object.values(FINDER_ACTION_LABEL),
      ...Object.values(FINDER_ACTION_FAILED),
      ...Object.values(FINDER_REASON_COPY).map((c) => c.sentence),
      ...Object.values(FINDER_STATUS_PILL),
    ]
    for (const text of all) expect(text).not.toMatch(/Add to Finder|\bInstall\b/)
  })

  test('the design artefact draws exactly this copy and no "Finder not added" state (spec §12)', () => {
    const html = readFileSync(new URL('../design/hifi/macos-menubar-popover.html', import.meta.url), 'utf8')
    expect(html).not.toContain('Finder not added')
    expect(html).toContain(FINDER_ADDING_LINE)
    for (const reason of FINDER_FAILURE_REASONS) {
      expect(html).toContain(`<span class="board-tag">${reason}</span><p>${FINDER_REASON_COPY[reason].sentence}</p>`)
      expect(html).toContain(`<span class="board-btn">${FINDER_ACTION_LABEL[FINDER_REASON_COPY[reason].action]}</span>`)
    }
  })
})

describe('copyFinderSetupDetails', () => {
  const previous = (globalThis as any).window
  afterEach(() => { (globalThis as any).window = previous })
  const backend = (answer: () => unknown) => {
    ;(globalThis as any).window = { __TAURI_INTERNALS__: { invoke: async (name: string) => (name === 'finder_setup_copy_details' ? answer() : undefined) } }
  }

  test('puts the command text on the pasteboard', async () => {
    backend(() => 'Beebeeb 0.8.12 on macOS 26.0')
    const written: string[] = []
    const result = await copyFinderSetupDetails({ writeClipboard: async (text) => { written.push(text) } })
    expect(result.ok).toBe(true)
    expect(written).toEqual(['Beebeeb 0.8.12 on macOS 26.0'])
  })

  test('a refused pasteboard is a plain failed result, for the surface to toast', async () => {
    backend(() => 'x')
    const result = await copyFinderSetupDetails({ writeClipboard: async () => { throw new Error('denied') } })
    expect(result).toEqual({ ok: false, reason: 'The details could not be put on the pasteboard.', unsupported: false })
  })
})
```

- [ ] **Step 2: Run RED**

```bash
cd $WT && bun test tests/finderSetupCopy.test.ts > $EVID/t14-red.log 2>&1; echo "rc=$?"; grep -E "Cannot find module|error:|pass|fail" $EVID/t14-red.log | head
```

Expected: `rc=1`, `Cannot find module '../src/finderSetup'`.

- [ ] **Step 3: Implement**

`src/finderSetup.ts`:

```ts
/**
 * macOS Finder setup (spec docs/specs/2026-10-06-macos-finder-setup-reconciler.md). The reconciler
 * in Rust (src-tauri/src/finder_setup/) owns the state. Surfaces read it (`finder_setup_state`),
 * follow `finder-setup-changed`, and offer the one action a state allows. No surface on macOS
 * installs anything (ruling R5). The shapes mirror `FinderSetupView` in finder_setup/driver.rs.
 */
import { listen } from '@tauri-apps/api/event'
import { command, type CommandResult } from './desktopApi'
import type { FinderSetupAction } from './finderSetupCopy'

export const FINDER_FAILURE_REASONS = [
  'extension_loading',
  'user_disabled',
  'not_in_applications',
  'folder_taken',
  'signing',
  'timeout',
  'unknown',
] as const
export type FinderFailureReason = (typeof FINDER_FAILURE_REASONS)[number]
export type FinderSetupState = 'ready' | 'missing' | 'adding' | 'failed' | 'user_disabled'
export type LaunchLocation = 'applications' | 'translocated' | 'disk_image' | 'elsewhere'

export interface FinderFailureRecord {
  reason: FinderFailureReason
  domain: string
  code: number
  at: number
}

export interface FinderSetupView {
  setup: FinderSetupState
  reason: FinderFailureReason | null
  launch_location: LaunchLocation
  attempt: number
  max_attempts: number
  last_failure: FinderFailureRecord | null
}

export const FINDER_SETUP_CHANGED_EVENT = 'finder-setup-changed'

/** The command each action sends, for an honest "not wired" label. */
export const FINDER_ACTION_COMMAND: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'finder_setup_retry',
  open_system_settings: 'open_login_items_and_extensions_settings',
  show_in_finder: 'finder_setup_show_app',
  copy_details: 'finder_setup_copy_details',
}

export function loadFinderSetup(): Promise<CommandResult<FinderSetupView>> {
  return command<FinderSetupView>('finder_setup_state')
}

/** Follow every reconciler transition. Returns the unsubscribe for an effect's cleanup. */
export function subscribeFinderSetup(onView: (view: FinderSetupView) => void): () => void {
  let closed = false
  let stop: (() => void) | null = null
  listen<FinderSetupView>(FINDER_SETUP_CHANGED_EVENT, (event) => {
    if (!closed) onView(event.payload)
  })
    .then((unlisten) => {
      if (closed) unlisten()
      else stop = unlisten
    })
    .catch(() => {
      // No event bus (a webview without Tauri): the surface keeps the view it loaded.
    })
  return () => {
    closed = true
    stop?.()
  }
}

export interface FinderActionDeps {
  writeClipboard?: (text: string) => Promise<void>
}

export async function copyFinderSetupDetails(deps: FinderActionDeps = {}): Promise<CommandResult<void>> {
  const details = await command<string>('finder_setup_copy_details')
  if (!details.ok) return details
  const write = deps.writeClipboard ?? ((text: string) => navigator.clipboard.writeText(text))
  try {
    await write(details.value)
    return { ok: true, value: undefined }
  } catch {
    return { ok: false, reason: 'The details could not be put on the pasteboard.', unsupported: false }
  }
}

export function runFinderSetupAction(action: FinderSetupAction, deps: FinderActionDeps = {}): Promise<CommandResult<void>> {
  switch (action) {
    case 'try_again':
      return command<void>('finder_setup_retry')
    case 'open_system_settings':
      return command<void>('open_login_items_and_extensions_settings')
    case 'show_in_finder':
      return command<void>('finder_setup_show_app')
    case 'copy_details':
      return copyFinderSetupDetails(deps)
  }
}
```

`src/finderSetupCopy.ts`:

```ts
/**
 * Every Finder-setup string on macOS (spec 2026-10-06 §6.2: "All copy lives in one frontend
 * module … keyed by reason"). Surfaces never write their own. One sentence and one action per
 * reason; "Try again" only inside a failure. Pinned by tests/finderSetupCopy.test.ts, which also
 * holds the design artefact to the same strings.
 */
import type { FinderFailureReason, FinderSetupState, FinderSetupView } from './finderSetup'

export type FinderSetupAction = 'try_again' | 'open_system_settings' | 'show_in_finder' | 'copy_details'

export const FINDER_SETUP_TITLE = 'Beebeeb in Finder'
export const FINDER_ADDING_LINE = 'Adding Beebeeb to Finder…'
export const FINDER_READY_LINE = 'Your vault appears under Locations in Finder.'

export const FINDER_REASON_COPY: Readonly<Record<FinderFailureReason, { sentence: string; action: FinderSetupAction }>> = {
  extension_loading: { sentence: 'macOS hasn’t finished loading Beebeeb’s Finder extension.', action: 'try_again' },
  user_disabled: { sentence: 'Beebeeb is turned off in System Settings.', action: 'open_system_settings' },
  not_in_applications: { sentence: 'Beebeeb is running from the disk image. Move it to Applications, then open it again.', action: 'show_in_finder' },
  folder_taken: {
    sentence: 'An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it.',
    action: 'copy_details',
  },
  signing: { sentence: 'This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io.', action: 'copy_details' },
  timeout: { sentence: 'macOS didn’t finish adding Beebeeb to Finder.', action: 'try_again' },
  unknown: { sentence: 'Beebeeb couldn’t be added to Finder.', action: 'try_again' },
}

export const FINDER_ACTION_LABEL: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'Try again',
  open_system_settings: 'Open System Settings',
  show_in_finder: 'Show in Finder',
  copy_details: 'Copy details',
}

/** Toast titles for an action that failed (a failed action gates nothing, so it is a toast). */
export const FINDER_ACTION_FAILED: Readonly<Record<FinderSetupAction, string>> = {
  try_again: 'Couldn’t try again',
  open_system_settings: 'Couldn’t open System Settings',
  show_in_finder: 'Couldn’t show Beebeeb in Finder',
  copy_details: 'Couldn’t copy the details',
}

/** The short pill on the compact Status page (spec B deletes that page). */
export const FINDER_STATUS_PILL: Readonly<Record<FinderSetupState, string>> = {
  ready: 'Installed',
  adding: 'Adding',
  failed: 'Setup blocked',
  user_disabled: 'Turned off',
  missing: 'Checking',
}

export type FinderSetupPresentation =
  | { kind: 'quiet'; line: '' }
  | { kind: 'adding'; line: string }
  | { kind: 'ready'; line: string }
  | {
      kind: 'notice'
      tone: 'alert' | 'status'
      reason: FinderFailureReason
      sentence: string
      action: FinderSetupAction
      actionLabel: string
    }

/** One state, one presentation. `quiet` = the instant before the first check (or no view yet). */
export function finderSetupPresentation(view: FinderSetupView | null): FinderSetupPresentation {
  if (!view) return { kind: 'quiet', line: '' }
  if (view.setup === 'ready') return { kind: 'ready', line: FINDER_READY_LINE }
  if (view.setup === 'adding') return { kind: 'adding', line: FINDER_ADDING_LINE }
  const reason: FinderFailureReason | null =
    view.setup === 'user_disabled' ? 'user_disabled' : view.setup === 'failed' ? (view.reason ?? 'unknown') : view.reason
  if (!reason) return { kind: 'quiet', line: '' }
  const copy = FINDER_REASON_COPY[reason]
  return {
    kind: 'notice',
    tone: reason === 'user_disabled' ? 'status' : 'alert',
    reason,
    sentence: copy.sentence,
    action: copy.action,
    actionLabel: FINDER_ACTION_LABEL[copy.action],
  }
}
```

- [ ] **Step 4: Run GREEN, typecheck and lint**

```bash
cd $WT && bun test tests/finderSetupCopy.test.ts > $EVID/t14-green.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t14-green.log
bunx tsc --noEmit -p . > $EVID/t14-tsc.log 2>&1; echo "tsc rc=$?"
bunx eslint src/finderSetup.ts src/finderSetupCopy.ts > $EVID/t14-eslint.log 2>&1; echo "eslint rc=$?"
```

Expected: `10 pass`, `0 fail`, and both other commands `rc=0`.

- [ ] **Step 5: Mutation check (spec §13.1 pins every string)**

Change `macOS didn’t finish adding Beebeeb to Finder.` to `macOS didn’t finish adding Beebeeb.` in `finderSetupCopy.ts`. Expected: `every reason has exactly …` and `the design artefact draws exactly this copy …` both fail. Restore it. Paste the failures into Notes.

- [ ] **Step 6: Commit**

```bash
cd $WT && git add src/finderSetup.ts src/finderSetupCopy.ts tests/finderSetupCopy.test.ts
git commit -m "finder setup (frontend): the reconciler's view, one reason-keyed copy module, artefact parity (spec A §6.2, §10)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src/finderSetup.ts src/finderSetupCopy.ts tests/finderSetupCopy.test.ts
git show --stat HEAD
```

---

## Task 15: Onboarding's Finder step on macOS

**Lane T.**

**Files:**
- Modify: `src/Onboarding.tsx`: imports, `OnboardingView`, new `MacFinderStep`, `FinderInstallStep` becomes Windows/Linux only, and the constant `USER_ENABLED_POLL_INTERVAL_MS` is deleted
- Create: `tests/onboardingFinderStep.test.tsx`

**Interfaces:**
- Consumes: Task 14 (`loadFinderSetup`, `subscribeFinderSetup`, `runFinderSetupAction`, `FINDER_ACTION_COMMAND`, `finderSetupPresentation`, `FINDER_SETUP_TITLE`, `FINDER_ADDING_LINE`, `FINDER_ACTION_FAILED`, `type FinderSetupAction`, `type FinderSetupView`).
- Produces: `function MacFinderStep({ onDone }: { onDone: () => void })` in `Onboarding.tsx` (named in the source-contract test).

- [ ] **Step 1: Write the test**

`tests/onboardingFinderStep.test.tsx`:

```tsx
/**
 * Spec 2026-10-06 §10, Onboarding: on macOS there is no install button; the step shows Adding,
 * advances by itself on Ready, and on Failed / UserDisabled shows the one notice and one action
 * from finderSetupCopy.ts. On Linux the old step is unchanged.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as finderInstallCard from '../src/finderInstallCard'
import * as finderSetup from '../src/finderSetup'
import * as copy from '../src/finderSetupCopy'
import { FINDER_FAILURE_REASONS, type FinderSetupView } from '../src/finderSetup'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const Card = loadComponent('Onboarding.tsx', 'Card', { React })
const FORBIDDEN = ['install_finder_location', 'continue_without_finder_location', 'finder_location_state', 'finder_domain_user_enabled']

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'adding',
  reason: null,
  launch_location: 'applications',
  attempt: 1,
  max_attempts: 0,
  last_failure: null,
  ...over,
})

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

function openMacStep(initial: FinderSetupView, backend: Record<string, (a: any) => unknown> = {}) {
  let push: (v: FinderSetupView) => void = () => { throw new Error('not subscribed') }
  let done = 0
  const copied: string[] = []
  const m = mount('Onboarding.tsx', 'MacFinderStep', {
    backend: { finder_setup_state: () => initial, finder_setup_copy_details: () => 'details', ...backend },
    props: { onDone: () => { done += 1 } },
    bindings: {
      ...desktopApi,
      ...finderSetup,
      ...copy,
      Card,
      subscribeFinderSetup: (cb: (v: FinderSetupView) => void) => { push = cb; return () => {} },
      runFinderSetupAction: (action: copy.FinderSetupAction) =>
        finderSetup.runFinderSetupAction(action, { writeClipboard: async (text) => { copied.push(text) } }),
    },
  })
  mounted.push(m)
  return { m, push: (v: FinderSetupView) => push(v), done: () => done, copied }
}

const buttons = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())
const notices = (m: Mounted) => m.elements().filter((el) => el.props.role === 'alert' || el.props.role === 'status')
const allStates = [view({ setup: 'missing' }), view(), view({ setup: 'ready' }), ...FINDER_FAILURE_REASONS.map((reason) => view({ setup: reason === 'user_disabled' ? 'user_disabled' : 'failed', reason }))]

describe('Onboarding Finder step on macOS', () => {
  test('shows Adding with no button, then advances by itself on a Ready event', async () => {
    const s = openMacStep(view())
    await s.m.flush()
    expect(textOf(s.m.tree())).toContain('Adding Beebeeb to Finder…')
    expect(buttons(s.m)).toEqual([])
    expect(s.done()).toBe(0)
    s.push(view({ setup: 'ready' }))
    await s.m.flush()
    expect(s.done()).toBe(1)
  })

  test('each reason renders exactly one notice and the action from §6.2', async () => {
    for (const reason of FINDER_FAILURE_REASONS) {
      const s = openMacStep(view({ setup: reason === 'user_disabled' ? 'user_disabled' : 'failed', reason }))
      await s.m.flush()
      expect(notices(s.m)).toHaveLength(1)
      expect(textOf(notices(s.m)[0].props.children)).toContain(copy.FINDER_REASON_COPY[reason].sentence)
      expect(buttons(s.m)).toEqual([copy.FINDER_ACTION_LABEL[copy.FINDER_REASON_COPY[reason].action]])
      expect(s.m.toasts).toEqual([])
    }
  })

  test('no Install or Add to Finder button in any state, and none of the install-era commands is called', async () => {
    for (const state of allStates) {
      const s = openMacStep(state)
      await s.m.flush()
      expect(buttons(s.m).join('|')).not.toMatch(/Install|Add to Finder/)
      expect(s.m.calls.map((c) => c.name).filter((name) => FORBIDDEN.includes(name))).toEqual([])
    }
  })

  test('Try again sends finder_setup_retry; Copy details puts the details on the pasteboard', async () => {
    const retry = openMacStep(view({ setup: 'failed', reason: 'timeout' }), { finder_setup_retry: () => undefined })
    await retry.m.flush()
    await retry.m.click('Try again')
    expect(retry.m.calls.filter((c) => c.name === 'finder_setup_retry')).toHaveLength(1)
    const details = openMacStep(view({ setup: 'failed', reason: 'folder_taken' }))
    await details.m.flush()
    await details.m.click('Copy details')
    expect(details.copied).toEqual(['details'])
  })

  test('a failed action is a toast, never a second inline surface', async () => {
    const s = openMacStep(view({ setup: 'failed', reason: 'timeout' }), { finder_setup_retry: () => { throw new Error('bridge down') } })
    await s.m.flush()
    await s.m.click('Try again')
    expect(s.m.toasts.map((t) => t.title)).toEqual(['Couldn’t try again'])
    expect(s.m.elements().filter((el) => el.props.role === 'alert')).toHaveLength(1)
  })
})

describe('Onboarding Finder step on Windows/Linux is unchanged', () => {
  test('Install Finder location sends the chosen path; a failure offers Continue without install', async () => {
    const m = mount('Onboarding.tsx', 'FinderInstallStep', {
      backend: {
        default_sync_root: () => '/home/sam/Beebeeb',
        finder_location_state: () => ({ installed: false, status: 'missing' }),
        install_finder_location: () => { throw new Error('File Provider is only available on macOS.') },
      },
      props: { onDone: () => {} },
      bindings: { ...desktopApi, ...finderInstallCard, Card },
    })
    mounted.push(m)
    await m.flush()
    await m.click('Install Finder location')
    expect(m.calls.find((c) => c.name === 'install_finder_location')?.args).toEqual({ path: '/home/sam/Beebeeb' })
    expect(buttons(m)).toContain('Continue without install')
  })
})
```

Run RED: `bun test tests/onboardingFinderStep.test.tsx > $EVID/t15-red.log 2>&1`. Expected: `Missing component MacFinderStep in Onboarding.tsx` (the harness error), so every macOS test fails.

- [ ] **Step 2: Implement in `src/Onboarding.tsx`**

1. Imports: delete `shouldRetryAfterUserEnabledPoll` from the `./finderInstallCard` import (keep `classifyFinderInstallResult`), and add:

```tsx
import { FINDER_ACTION_COMMAND, loadFinderSetup, runFinderSetupAction, subscribeFinderSetup, type FinderSetupView } from './finderSetup'
import { FINDER_ACTION_FAILED, FINDER_ADDING_LINE, FINDER_SETUP_TITLE, finderSetupPresentation, type FinderSetupAction } from './finderSetupCopy'
```

Delete `USER_ENABLED_POLL_INTERVAL_MS` and its comment (lines 21–25).

2. `OnboardingView`: add `const [platform, setPlatform] = useState<DesktopPlatform>('unknown')`, and replace the mount effect with:

```tsx
  useEffect(() => {
    let cancelled = false

    Promise.all([loadSyncStatus(), command<DesktopPlatform>('desktop_platform')]).then(async ([status, platformResult]) => {
      if (cancelled) return
      if (platformResult.ok) setPlatform(platformResult.value)
      if (!status?.logged_in) return

      if (!status.vault_unlocked) {
        setStep('unlock')
        return
      }

      if (platformResult.ok && platformResult.value === 'macos') {
        // Spec 2026-10-06 §10: the reconciler's state, not the install-era command.
        const finder = await loadFinderSetup()
        if (cancelled) return
        setStep(finder.ok && finder.value.setup === 'ready' ? 'ready' : 'finder')
        return
      }
      setStep(status.sync_root ? 'ready' : 'finder')
    })

    return () => {
      cancelled = true
    }
  }, [])
```

Then replace `{step === 'finder' && <FinderInstallStep onDone={() => setStep('pinning')} />}` with:

```tsx
        {step === 'finder' && platform !== 'unknown' && (platform === 'macos' ? <MacFinderStep onDone={() => setStep('pinning')} /> : <FinderInstallStep onDone={() => setStep('pinning')} />)}
```

3. Add `MacFinderStep` directly above `FinderInstallStep`:

```tsx
/**
 * macOS (spec 2026-10-06 §10, ruling R5): no install button. Beebeeb adds itself once the keys
 * arrive; this step shows Adding, advances by itself on Ready, and on a failure or a turned-off
 * extension shows the one notice and the one action of finderSetupCopy.ts. The notice gates the
 * step, so it is inline; a failed ACTION gates nothing, so it is a toast.
 */
function MacFinderStep({ onDone }: { onDone: () => void }) {
  const { showToast } = useToast()
  const [view, setView] = useState<FinderSetupView | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    void loadFinderSetup().then((result) => {
      if (!cancelled && result.ok) setView(result.value)
    })
    const unsubscribe = subscribeFinderSetup((next) => {
      if (!cancelled) setView(next)
    })
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [])

  useEffect(() => {
    if (view?.setup === 'ready') onDone()
  }, [view, onDone])

  const presentation = finderSetupPresentation(view)

  const act = async (action: FinderSetupAction) => {
    setBusy(true)
    const result = await runFinderSetupAction(action)
    setBusy(false)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: FINDER_ACTION_FAILED[action],
        message: result.unsupported ? commandUnavailableLabel(FINDER_ACTION_COMMAND[action]) : result.reason,
      })
    }
  }

  return (
    <Card title={FINDER_SETUP_TITLE} copy="Beebeeb appears as a system-managed Finder location. Offline folders are controlled separately.">
      {presentation.kind === 'notice' ? (
        <div
          className={presentation.tone === 'alert' ? 'notice error' : 'notice'}
          role={presentation.tone === 'alert' ? 'alert' : 'status'}
          data-error-surface={presentation.tone === 'alert' ? 'finder-setup' : undefined}
          style={{ marginTop: 16 }}
        >
          <div>{presentation.sentence}</div>
          <div className="button-row" style={{ marginTop: 10 }}>
            <button className="button" onClick={() => void act(presentation.action)} disabled={busy}>
              {presentation.actionLabel}
            </button>
          </div>
        </div>
      ) : (
        <div className="panel" style={{ marginTop: 16, background: 'var(--paper-2)' }}>
          <div className="mono" style={{ fontSize: 13 }}>
            {presentation.kind === 'quiet' ? FINDER_ADDING_LINE : presentation.line}
          </div>
        </div>
      )}
    </Card>
  )
}
```

4. Make `FinderInstallStep` Windows/Linux only. Replace the whole function with the body below. The comment block `KEPT INLINE BY DESIGN — task 1255 …` above `message` stays verbatim. The removed parts (platform state, `userDisabled`, its poll, `openSystemSettings`, the `isMacos` branches) only ever rendered on macOS.

```tsx
/**
 * Windows/Linux only since spec 2026-10-06 (macOS renders MacFinderStep). The macOS-only
 * branches (the turned-off card, its userEnabled poll, the System Settings link, the macOS copy)
 * are gone; on these platforms they never rendered.
 */
function FinderInstallStep({ onDone }: { onDone: () => void }) {
  const [syncRoot, setSyncRoot] = useState<string | null>(null)
  const [finderPath, setFinderPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // (keep the existing "KEPT INLINE BY DESIGN — task 1255" comment block here, verbatim)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(() => {
    command<string>('default_sync_root').then((result) => {
      if (result.ok) setSyncRoot(result.value)
    })
    command<FinderInstallState>('finder_location_state').then((result) => {
      if (result.ok) setFinderPath(result.value.path ?? null)
    })
  }, [])

  const chooseFolder = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const picked = await command<string | null>('pick_sync_root')
    setBusy(false)
    if (picked.ok && picked.value) {
      setSyncRoot(picked.value)
      return
    }
    if (!picked.ok) setMessage(picked.unsupported ? commandUnavailableLabel('pick_sync_root') : picked.reason)
  }, [])

  const install = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<FinderInstallState>('install_finder_location', { path: syncRoot })
    setBusy(false)
    const outcome = classifyFinderInstallResult(result)
    if (outcome.kind === 'installed') {
      setFinderPath(outcome.path)
      onDone()
      return
    }
    setMessage(!result.ok && result.unsupported ? commandUnavailableLabel('install_finder_location') : outcome.message)
  }, [onDone, syncRoot])

  const continueWithoutInstall = useCallback(async () => {
    setBusy(true)
    setMessage(null)
    const result = await command<void>('continue_without_finder_location', { path: syncRoot })
    setBusy(false)
    if (result.ok) {
      onDone()
      return
    }
    setMessage(result.unsupported ? commandUnavailableLabel('continue_without_finder_location') : result.reason)
  }, [onDone, syncRoot])

  return (
    <Card
      title="Install the Finder location"
      copy="Beebeeb should appear as a file-manager location. This is separate from choosing optional offline folders."
    >
      {message && <div className="notice">{message}</div>}
      <div className="panel" style={{ marginTop: 16, background: 'var(--paper-2)' }}>
        <div className="section-label">Folder path</div>
        <div className="mono" style={{ marginTop: 8, fontSize: 13 }}>
          {finderPath ?? syncRoot ?? '~/Beebeeb'}
        </div>
      </div>
      <div className="button-row" style={{ marginTop: 16 }}>
        <button className="button" onClick={chooseFolder} disabled={busy}>
          Choose location
        </button>
        <button className="button amber" onClick={install} disabled={busy}>
          {busy ? 'Installing…' : 'Install Finder location'}
        </button>
        {/* This escape hatch EXISTS ONLY while `message` is set — it is the gate described on the
            `message` state above. Removing the inline error removes this button. */}
        {message && (
          <button className="button" onClick={continueWithoutInstall} disabled={busy}>
            Continue without install
          </button>
        )}
      </div>
    </Card>
  )
}
```

Add `useToast` to the Onboarding imports only if it is not already imported (it is: `import { useToast } from './windows/ui'`).

- [ ] **Step 3: Run GREEN + typecheck + lint**

```bash
cd $WT && bun test tests/onboardingFinderStep.test.tsx > $EVID/t15-green.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t15-green.log
bunx tsc --noEmit -p . > $EVID/t15-tsc.log 2>&1; echo "tsc rc=$?"
bunx eslint src/Onboarding.tsx > $EVID/t15-eslint.log 2>&1; echo "eslint rc=$?"
bun test tests/noAdHocErrorSurface.test.ts tests/brandResidencyCopy.test.ts tests/noHardcodedThemeColors.test.ts > $EVID/t15-neighbours.log 2>&1; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t15-neighbours.log
```

Expected: `6 pass`, `0 fail`. tsc and eslint give `rc=0`. The three neighbouring suites read `Onboarding.tsx` and must still pass with their previous counts.

- [ ] **Step 4: Mutation check**

In `MacFinderStep`, delete the `useEffect` that calls `onDone()`. Expected: `shows Adding … advances by itself on a Ready event` fails (`expected 1, received 0`). Restore it.

- [ ] **Step 5: Commit**

```bash
cd $WT && git add tests/onboardingFinderStep.test.tsx
git commit -m "onboarding (macOS): Beebeeb adds itself; the step shows Adding, advances on Ready, one notice per reason (spec A §10)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src/Onboarding.tsx tests/onboardingFinderStep.test.tsx
git show --stat HEAD
```

---

## Task 16: The Settings Sync tab and the Repair copy

**Lane T.**

**Files:**
- Modify: `src/macSettingsModel.ts` (`finderRow`, `finderHint`, `REPAIR_BODY`; delete `FINDER_FAILURE_TITLE`, `finderFailureCopy`; keep `monoReason`, `MONO_REASON_MAX`)
- Modify: `src/MacSettings.tsx` (`SyncTab`)
- Modify: `tests/macSettingsModel.test.ts`, `tests/macSettingsTabs.test.tsx`
- Modify: `tests/render-mac-settings.mjs` (local Playwright evidence script; the fixtures move to the new view)

**Interfaces:**
- Consumes: Task 14.
- Produces: `type FinderRow = { kind: 'loading' } | { kind: 'unavailable' } | { kind: 'adding' } | { kind: 'added' } | { kind: 'notice'; tone: 'alert' | 'status'; reason: FinderFailureReason; sentence: string; action: FinderSetupAction; actionLabel: string }`, `finderRow(view: FinderSetupView | null, loadFailed?: boolean): FinderRow`, `finderHint(row): string`.

- [ ] **Step 1: Write the tests**

In `tests/macSettingsModel.test.ts`, remove `FINDER_FAILURE_TITLE` and `finderFailureCopy` from the import, and replace the `describe('Beebeeb in Finder', …)` block (lines 93–136) and the REPAIR assertions (lines ~138–141) with:

```ts
describe('Beebeeb in Finder (spec 2026-10-06)', () => {
  const v = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
    setup: 'missing', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null, ...over,
  })

  test('one row per reconciler state, and never a "missing" row with an add button (R5)', () => {
    expect(finderRow(null)).toEqual({ kind: 'loading' })
    expect(finderRow(null, true)).toEqual({ kind: 'unavailable' })
    expect(finderRow(v({ setup: 'ready' }))).toEqual({ kind: 'added' })
    expect(finderRow(v({ setup: 'adding' }))).toEqual({ kind: 'adding' })
    expect(finderRow(v({ setup: 'missing' }))).toEqual({ kind: 'adding' })
    expect(finderRow(v({ setup: 'failed', reason: 'folder_taken' }))).toEqual({
      kind: 'notice', tone: 'alert', reason: 'folder_taken',
      sentence: FINDER_REASON_COPY.folder_taken.sentence, action: 'copy_details', actionLabel: 'Copy details',
    })
    expect(finderRow(v({ setup: 'user_disabled', reason: 'user_disabled' }))).toMatchObject({ kind: 'notice', tone: 'status', action: 'open_system_settings' })
  })

  test('the hint claims the vault is in Finder only once it is', () => {
    expect(finderHint({ kind: 'added' })).toBe('Your vault appears under Locations in Finder.')
    expect(finderHint({ kind: 'adding' })).toBe('Adding Beebeeb to Finder…')
    expect(finderHint({ kind: 'loading' })).toBe('')
    expect(finderHint({ kind: 'unavailable' })).toBe('Couldn’t check Finder.')
    expect(finderHint(finderRow(v({ setup: 'failed', reason: 'timeout' })))).toBe('')
  })

  test('Repair says what it does, names the login item, and that Beebeeb comes back by itself', () => {
    expect(REPAIR_TITLE).toBe('Repair Beebeeb in Finder?')
    expect(REPAIR_BODY).toBe(
      'Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.',
    )
  })
})
```

Add `import { FINDER_REASON_COPY } from '../src/finderSetupCopy'` and `import type { FinderSetupView } from '../src/finderSetup'`. The old `FinderInstallState` import stays only if other tests in the file still use it. Remove it if `bunx tsc` reports it unused.

In `tests/macSettingsTabs.test.tsx`:

(a) Replace the `TIMEOUT_TEXT` and `finder` fixtures (lines 96–101) with:

```ts
const finderView = (over: Record<string, unknown> = {}) => ({
  setup: 'missing', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null, ...over,
})
const finder = {
  installed: finderView({ setup: 'ready' }),
  adding: finderView({ setup: 'adding' }),
  failed: finderView({ setup: 'failed', reason: 'timeout', attempt: 4, max_attempts: 4 }),
  folderTaken: finderView({ setup: 'failed', reason: 'folder_taken' }),
  userDisabled: finderView({ setup: 'user_disabled', reason: 'user_disabled' }),
}
```

(b) Add `...finderSetup, ...finderSetupCopy` (namespace imports of `../src/finderSetup` and `../src/finderSetupCopy`) to `baseBindings`, and add a bus. Every `open(...)` passes `subscribeFinderSetup` through `extra.bindings`:

```ts
function finderBus() {
  let push: ((v: unknown) => void) | null = null
  return {
    bindings: { subscribeFinderSetup: (cb: (v: unknown) => void) => { push = cb; return () => {} } },
    push: (v: unknown) => push?.(v),
  }
}
```

(c) Replace `syncBackend`, `openSync` and the Sync-tab Finder tests (from `function syncBackend` through the test `a successful repair whose refreshed Finder state cannot be read …`) with:

```ts
  function syncBackend(opts: { finder?: any; tree?: any[]; repair?: (a: any) => unknown; pin?: (a: any) => unknown; retry?: () => unknown } = {}) {
    const st = { finder: opts.finder ?? finder.installed }
    return {
      st,
      backend: {
        finder_setup_state: () => st.finder,
        finder_setup_retry: opts.retry ?? (() => undefined),
        finder_setup_copy_details: () => 'details',
        finder_setup_show_app: () => undefined,
        reset_macos_integration: (a: any) => {
          const result = opts.repair ? opts.repair(a) : { removed_file_provider_domain: true, disabled_autostart: true, removed_socket: true, removed_cache_files: 0, skipped_cache_files: 0, pending_operations_preserved: 0, warnings: [] }
          st.finder = finder.adding // the reconciler re-adds by itself
          return result
        },
        list_remote_tree: () => opts.tree ?? [folder('a', 'Photos', true), folder('b', 'Work', false)],
        set_recursive_pin: opts.pin ?? (() => undefined),
        open_login_items_and_extensions_settings: () => undefined,
      } as Record<string, (a: any) => unknown>,
    }
  }
  const openSync = async (opts: Parameters<typeof syncBackend>[0] = {}, settings: any = ready()) => {
    const { backend, st } = syncBackend(opts)
    const bus = finderBus()
    const m = open('SyncTab', backend, { props: { settings }, bindings: bus.bindings })
    await m.flush()
    return { m, st, bus }
  }

  test('an added Finder location reads Added with Repair…, no error, and the ready hint', async () => {
    const { m } = await openSync()
    expect(visibleText(m)).toContain('Your vault appears under Locations in Finder.')
    expect(visibleText(m)).toContain('Added')
    expect(buttons(m)).toContain('Repair…')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('no state ever offers Add to Finder or Install (R5)', async () => {
    for (const state of Object.values(finder)) {
      const { m } = await openSync({ finder: state })
      expect(buttons(m).join('|')).not.toMatch(/Add to Finder|Install/)
      expect(m.calls.map((c) => c.name)).not.toContain('install_finder_location')
      expect(m.calls.map((c) => c.name)).not.toContain('finder_location_state')
    }
  })

  test('adding is a disabled Adding… with the adding line, and no error', async () => {
    const { m } = await openSync({ finder: finder.adding })
    expect(button(m, 'Adding…').props.disabled).toBe(true)
    expect(visibleText(m)).toContain('Adding Beebeeb to Finder…')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('a failure is ONE inline alert with one sentence and one action, never a toast, never raw error text', async () => {
    const { m } = await openSync({ finder: finder.failed })
    expect(find(m, (el) => el.props['data-error-surface'] === 'finder-setup')).toHaveLength(1)
    expect(alerts(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
    expect(visibleText(m)).toContain('macOS didn’t finish adding Beebeeb to Finder.')
    expect(visibleText(m)).toContain('reason: timeout')
    expect(buttons(m).filter((b) => b === 'Try again')).toHaveLength(1)
    expect(visibleText(m)).not.toContain('File Provider')
  })

  test('Try again sends finder_setup_retry once; folder_taken offers Copy details and no Try again', async () => {
    const { m } = await openSync({ finder: finder.failed })
    await press(m, 'Try again')
    expect(m.calls.filter((c) => c.name === 'finder_setup_retry')).toHaveLength(1)
    const taken = await openSync({ finder: finder.folderTaken })
    expect(buttons(taken.m)).toContain('Copy details')
    expect(buttons(taken.m)).not.toContain('Try again')
  })

  test('a turned-off extension is a neutral notice with Open System Settings, not an error', async () => {
    const { m } = await openSync({ finder: finder.userDisabled })
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(statuses(m)).toHaveLength(1)
    await press(m, 'Open System Settings')
    expect(m.calls.filter((c) => c.name === 'open_login_items_and_extensions_settings')).toHaveLength(1)
  })

  test('the row follows finder-setup-changed without asking again (no polling)', async () => {
    const { m, bus } = await openSync({ finder: finder.adding })
    const reads = m.calls.filter((c) => c.name === 'finder_setup_state').length
    bus.push(finder.installed)
    await m.flush()
    expect(buttons(m)).toContain('Repair…')
    expect(m.calls.filter((c) => c.name === 'finder_setup_state')).toHaveLength(reads)
  })

  test('a failed Finder action is a toast and does not add a second surface', async () => {
    const { m } = await openSync({ finder: finder.failed, retry: () => { throw new Error('bridge down') } })
    await press(m, 'Try again')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t try again'])
    expect(alerts(m)).toHaveLength(1)
  })
```

Keep the three Repair tests that follow (`Repair… asks first …`, `confirming a repair resets once …`, `a repair that fails is ONE inline alert …`), with these edits. In `confirming a repair …`, replace `finder_location_state` with `finder_setup_state` in the count assertion. Replace `expect(buttons(m)).toContain('Add to Finder') // the repair removed it…` with `expect(button(m, 'Adding…').props.disabled).toBe(true) // the reconciler adds it back by itself`. In `a successful repair whose refreshed …`, rename the backend key `finder_location_state` to `finder_setup_state`, set `st.finder = finder.adding` in its `reset_macos_integration`, and pass `bindings: finderBus().bindings` to `open`. At line ~970 (the whole-window backend), replace `finder_location_state: () => finder.installed, install_finder_location: () => finder.installed,` with `finder_setup_state: () => finder.installed,`.

Run RED: `bun test tests/macSettingsModel.test.ts tests/macSettingsTabs.test.tsx > $EVID/t16-red.log 2>&1`. Expected: failures in the new Finder tests (the old `finderRow` signature, `Add to Finder` present, `finder_setup_state` unscripted → "unscripted command finder_location_state").

- [ ] **Step 2: Implement**

`src/macSettingsModel.ts`. Replace from `export const FINDER_FAILURE_TITLE` through the end of `finderHint`. `monoReason` and `MONO_REASON_MAX` above it stay.

```ts
export type FinderRow =
  | { kind: 'loading' }
  | { kind: 'unavailable' }
  | { kind: 'adding' }
  | { kind: 'added' }
  | { kind: 'notice'; tone: 'alert' | 'status'; reason: FinderFailureReason; sentence: string; action: FinderSetupAction; actionLabel: string }

/**
 * One row, one reconciler state (spec 2026-10-06). `loadFailed` is a LOAD failure (the state could
 * not be read at all). A failed setup is part of the state and is shown once, under the row. The
 * instant before the first check reads as Adding: there is no "missing" row with an add button (R5).
 */
export function finderRow(view: FinderSetupView | null, loadFailed = false): FinderRow {
  if (view === null) return loadFailed ? { kind: 'unavailable' } : { kind: 'loading' }
  const presentation = finderSetupPresentation(view)
  if (presentation.kind === 'ready') return { kind: 'added' }
  if (presentation.kind === 'notice') {
    const { tone, reason, sentence, action, actionLabel } = presentation
    return { kind: 'notice', tone, reason, sentence, action, actionLabel }
  }
  return { kind: 'adding' }
}

/** The hint under "Beebeeb in Finder". A notice row says its sentence in the Note, not here. */
export function finderHint(row: FinderRow): string {
  switch (row.kind) {
    case 'added':
      return FINDER_READY_LINE
    case 'adding':
      return FINDER_ADDING_LINE
    case 'unavailable':
      return 'Couldn’t check Finder.'
    default:
      return ''
  }
}
```

Imports at the top of `macSettingsModel.ts`: `import type { FinderFailureReason, FinderSetupView } from './finderSetup'` and `import { FINDER_ADDING_LINE, FINDER_READY_LINE, finderSetupPresentation, type FinderSetupAction } from './finderSetupCopy'`. Remove the `finderInstallNotice` import if nothing else uses it. Then:

```ts
export const REPAIR_BODY =
  'Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.'
```

`src/MacSettings.tsx`, `SyncTab`:

- Imports: remove `finderInstallStateAfterAttempt, finderInstallStateWhileAttempting` and `type FinderInstallState`; add `import { FINDER_ACTION_COMMAND, loadFinderSetup, runFinderSetupAction, subscribeFinderSetup, type FinderSetupView } from './finderSetup'` and `import { FINDER_ACTION_FAILED, type FinderSetupAction } from './finderSetupCopy'`; import `monoReason` from `./macSettingsModel` next to `finderRow`.
- State: `const [finder, setFinder] = useState<FinderSetupView | null>(null)`. Delete `attempting`.
- `loadFinder` calls `loadFinderSetup()`; the rest is unchanged.
- After the existing mount effect, add:

```tsx
  // finder-setup-changed replaces polling: every reconciler transition re-renders the row.
  useEffect(
    () =>
      subscribeFinderSetup((view) => {
        setFinderLoadFailed(false)
        setFinder(view)
      }),
    [],
  )
```

- Delete `addToFinder` and `openSystemSettings`, and add:

```tsx
  // A one-off action that gates nothing: a failure is a toast (house rule).
  const runFinderAction = async (action: FinderSetupAction) => {
    const result = await runFinderSetupAction(action)
    if (!result.ok) {
      showToast({
        variant: 'error',
        title: FINDER_ACTION_FAILED[action],
        message: result.unsupported ? commandUnavailableLabel(FINDER_ACTION_COMMAND[action]) : result.reason,
      })
    }
  }
```

- `const row = finderRow(finder, finderLoadFailed)`. The control: `added` keeps its fragment (Added + Repair…), `adding` → `<Btn disabled>Adding…</Btn>`, `unavailable` → `<Btn onClick={() => void loadFinder()}>Try again</Btn>`, and nothing for `notice`/`loading`.
- Replace the two Finder `Note` blocks (`row.kind === 'failed'` and `row.kind === 'user_disabled'`) with one:

```tsx
        {row.kind === 'notice' ? (
          <Note
            kind={row.tone}
            surface="finder-setup"
            reason={row.tone === 'alert' ? monoReason(row.reason) : null}
            actions={<Btn onClick={() => void runFinderAction(row.action)}>{row.actionLabel}</Btn>}
          >
            {row.sentence}
          </Note>
        ) : null}
```

- In `runRepair`, delete `setRepairFailed(false)`'s neighbour `setAttempting` if one is referenced. The rest is unchanged: it still calls `loadFinder()` after the repair.

`tests/render-mac-settings.mjs`: replace the five Finder fixture literals at lines 61–67 with views built the same way as `finderView` above (`installed` → `setup: 'ready'`; `failed` → `setup: 'failed', reason: 'timeout'`; `failedLongCategory` → `setup: 'failed', reason: 'not_in_applications'`; `userDisabled` → `setup: 'user_disabled', reason: 'user_disabled'`; the missing one → `setup: 'adding'`). In the synthetic IPC switch (lines ~131–133), replace the `finder_location_state` and `install_finder_location` cases with `case 'finder_setup_state': return f.finder`, and make `reset_macos_integration` set `f.finder` to the adding view. This script is local evidence (not CI). Run it only if Playwright is available (Task 21).

- [ ] **Step 3: Run GREEN + typecheck + lint**

```bash
cd $WT && bun test tests/macSettingsModel.test.ts tests/macSettingsTabs.test.tsx > $EVID/t16-green.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t16-green.log
bunx tsc --noEmit -p . > $EVID/t16-tsc.log 2>&1; echo "tsc rc=$?"
bunx eslint src/MacSettings.tsx src/macSettingsModel.ts > $EVID/t16-eslint.log 2>&1; echo "eslint rc=$?"
```

Expected: `0 fail`, and tsc and eslint give `rc=0`. Write the pass count, compared with the baseline counts of these two files, into Notes.

- [ ] **Step 4: Mutation check**

In `finderRow`, return `{ kind: 'missing' } as any` for `view.setup === 'missing'`. Expected: `one row per reconciler state …` fails. Restore it.

- [ ] **Step 5: Commit**

```bash
cd $WT && git commit -m "settings (macOS): the Finder row follows the reconciler; one notice + one action; Repair adds Beebeeb back (spec A §10, R5)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src/MacSettings.tsx src/macSettingsModel.ts tests/macSettingsModel.test.ts tests/macSettingsTabs.test.tsx tests/render-mac-settings.mjs
git show --stat HEAD
```

---

## Task 17: SyncFolder, Status and the macOS main-window panel; the source contract

**Lane T.**

**Files:**
- Modify: `src/pages/SyncFolder.tsx`, `src/pages/Status.tsx`, `src/windows/views/SettingsView.tsx`, `src/finderInstallCard.ts`
- Modify: `tests/finderInstallOneSurface.test.tsx`, `tests/finderInstallCard.test.ts`
- Create: `tests/statusFinderSetup.test.tsx`, `tests/finderSetupSourceContract.test.ts`

**Interfaces:**
- Consumes: Task 14.
- Produces: `function MacFinderIntegrationPanel()` and `function ShellOrFinderPanel({ status })` in `SettingsView.tsx`; `function macFinderSetupState(view: FinderSetupView | null)` in `pages/Status.tsx`.

- [ ] **Step 1: Write the tests**

(a) `tests/finderSetupSourceContract.test.ts`:

```ts
/**
 * Spec 2026-10-06 §13.1: "a source-contract test that no macOS code path calls
 * install_finder_location" — extended to every install-era Finder command. Static for the
 * macOS-only units, a census for everything else; the mixed pages are proven by behaviour
 * (tests/statusFinderSetup.test.tsx, the macOS describe in tests/finderInstallOneSurface.test.tsx).
 */
import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'
import ts from 'typescript'

const SRC = new URL('../src/', import.meta.url).pathname
const FORBIDDEN_ON_MACOS = ['install_finder_location', 'continue_without_finder_location', 'finder_location_state', 'finder_domain_user_enabled']

function functionText(file: string, name: string): string {
  const source = readFileSync(join(SRC, file), 'utf8')
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const found = ast.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)
  if (!found) throw new Error(`${name} not found in ${file}`)
  return found.getText(ast)
}

function allSources(dir = SRC, out = new Map<string, string>()): Map<string, string> {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) allSources(path, out)
    else if (/\.(ts|tsx)$/.test(entry)) out.set(relative(SRC, path), readFileSync(path, 'utf8'))
  }
  return out
}

describe('no macOS code path calls the install-era Finder commands (R5)', () => {
  test('macOS-only units never name them', () => {
    const units: Array<[string, string | null]> = [
      ['MacSettings.tsx', null],
      ['macSettingsModel.ts', null],
      ['finderSetup.ts', null],
      ['finderSetupCopy.ts', null],
      ['Onboarding.tsx', 'MacFinderStep'],
      ['windows/views/SettingsView.tsx', 'MacFinderIntegrationPanel'],
      ['pages/Status.tsx', 'macFinderSetupState'],
    ]
    for (const [file, fn] of units) {
      const text = fn ? functionText(file, fn) : readFileSync(join(SRC, file), 'utf8')
      const named = FORBIDDEN_ON_MACOS.filter((command) => text.includes(command))
      expect({ unit: `${file}${fn ? `#${fn}` : ''}`, named }).toEqual({ unit: `${file}${fn ? `#${fn}` : ''}`, named: [] })
    }
  })

  test('outside them, only the Windows/Linux branches that keep these commands name them', () => {
    const allowed: Record<string, string[]> = {
      install_finder_location: ['Onboarding.tsx', 'pages/SyncFolder.tsx'],
      continue_without_finder_location: ['Onboarding.tsx'],
      finder_location_state: ['Onboarding.tsx', 'pages/Status.tsx', 'pages/SyncFolder.tsx'],
      finder_domain_user_enabled: [],
    }
    const sources = [...allSources()]
    for (const [command, files] of Object.entries(allowed)) {
      const named = sources.filter(([, text]) => text.includes(`'${command}'`)).map(([file]) => file).sort()
      expect({ command, named }).toEqual({ command, named: [...files].sort() })
    }
  })

  test('Onboarding routes macOS to MacFinderStep; the old step is not reachable there', () => {
    expect(functionText('Onboarding.tsx', 'OnboardingView')).toMatch(/platform === 'macos' \? <MacFinderStep/)
    expect(functionText('Onboarding.tsx', 'FinderInstallStep')).not.toContain('finder_domain_user_enabled')
  })

  test('the main-window Settings panel dispatches macOS to MacFinderIntegrationPanel', () => {
    expect(functionText('windows/views/SettingsView.tsx', 'ShellOrFinderPanel')).toMatch(/name === 'macos' \? <MacFinderIntegrationPanel/)
    expect(functionText('windows/views/SettingsView.tsx', 'shellIntegrationCommandsFor')).not.toContain('finder_')
  })
})
```

(b) `tests/statusFinderSetup.test.tsx`:

```tsx
/** Spec §10: on macOS the Status page reads finder_setup_state and follows finder-setup-changed
 *  instead of polling finder_location_state every 3 s. Linux is unchanged. */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as finderSetup from '../src/finderSetup'
import * as copy from '../src/finderSetupCopy'
import type { FinderSetupView } from '../src/finderSetup'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const view = (over: Partial<FinderSetupView> = {}): FinderSetupView => ({
  setup: 'adding', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 0, last_failure: null, ...over,
})
const finderSetupState = loadComponent('pages/Status.tsx', 'finderSetupState', {})
const macFinderSetupState = loadComponent('pages/Status.tsx', 'macFinderSetupState', { ...copy })

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

function openStatus(platform: string, finder: unknown) {
  let push: (v: FinderSetupView) => void = () => {}
  const m = mount('pages/Status.tsx', 'Status', {
    backend: {
      desktop_platform: () => platform,
      sync_status: () => ({ logged_in: false, engine: 'stopped', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }),
      finder_setup_state: () => finder,
      finder_location_state: () => finder,
    },
    bindings: {
      ...desktopApi, ...finderSetup, ...copy, finderSetupState, macFinderSetupState,
      subscribeFinderSetup: (cb: (v: FinderSetupView) => void) => { push = cb; return () => {} },
    },
  })
  mounted.push(m)
  return { m, push: (v: FinderSetupView) => push(v) }
}

describe('Status page, Finder row', () => {
  test('macOS reads the reconciler, never finder_location_state, and follows the event', async () => {
    const { m, push } = openStatus('macos', view())
    await m.flush(); await m.flush()
    expect(m.calls.map((c) => c.name)).not.toContain('finder_location_state')
    expect(m.calls.filter((c) => c.name === 'finder_setup_state')).toHaveLength(1)
    expect(textOf(m.tree())).toContain('Adding')
    push(view({ setup: 'ready' }))
    await m.flush()
    expect(textOf(m.tree())).toContain('Installed')
    expect(m.calls.filter((c) => c.name === 'finder_setup_state')).toHaveLength(1)
  })

  test('macOS: a failure shows its §6.2 sentence', async () => {
    const { m } = openStatus('macos', view({ setup: 'failed', reason: 'folder_taken' }))
    await m.flush(); await m.flush()
    expect(textOf(m.tree())).toContain(copy.FINDER_REASON_COPY.folder_taken.sentence)
  })

  test('Linux is unchanged: it still reads finder_location_state', async () => {
    const { m } = openStatus('linux', { installed: false, status: 'missing' })
    await m.flush(); await m.flush()
    expect(m.calls.map((c) => c.name)).toContain('finder_location_state')
    expect(m.calls.map((c) => c.name)).not.toContain('finder_setup_state')
  })
})
```

(c) `tests/finderInstallOneSurface.test.tsx`:

- In `finderBackend`, change `desktop_platform: () => 'macos'` to `desktop_platform: () => 'linux'`, and rename the describe to `'SyncFolder on Windows/Linux (unchanged): …'`. These eight tests are now the proof that the non-macOS pane is unchanged. The test `success shows no error surface and swaps Install for Open in Finder` keeps asserting `Open in Finder` exists, which is true on Linux, where both buttons always show. The `windows` folder-picker test is unchanged.
- Add a macOS describe for SyncFolder:

```tsx
describe('SyncFolder on macOS follows the reconciler (spec §10)', () => {
  const v = (over: Record<string, unknown> = {}) => ({ setup: 'adding', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 0, last_failure: null, ...over })
  async function openMac(view: unknown) {
    const m = mount('pages/SyncFolder.tsx', 'SyncFolder', {
      backend: {
        desktop_platform: () => 'macos',
        sync_status: () => ({ logged_in: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 }),
        finder_setup_state: () => view,
        finder_setup_retry: () => undefined,
        open_finder_location: () => undefined,
      },
      bindings: { ...desktopApi, ...finderInstallCard, ...finderSetup, ...finderSetupCopy, subscribeFinderSetup: () => () => {} },
    })
    mounted.push(m)
    await m.flush(); await m.flush()
    return m
  }
  const btns = (m: Mounted) => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children).trim())

  test('no Install button in any state; Open in Finder only when Beebeeb is in Finder', async () => {
    for (const state of [v(), v({ setup: 'ready' }), v({ setup: 'failed', reason: 'timeout' }), v({ setup: 'user_disabled', reason: 'user_disabled' })]) {
      const m = await openMac(state)
      expect(btns(m).join('|')).not.toMatch(/Install/)
      expect(btns(m).includes('Open in Finder')).toBe((state as any).setup === 'ready')
      expect(m.calls.map((c) => c.name).filter((n) => n === 'install_finder_location' || n === 'finder_location_state')).toEqual([])
    }
  })

  test('a failure is one inline alert with Try again only there', async () => {
    const m = await openMac(v({ setup: 'failed', reason: 'timeout' }))
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(btns(m).filter((b) => b === 'Try again')).toHaveLength(1)
    await m.click('Try again')
    expect(m.calls.filter((c) => c.name === 'finder_setup_retry')).toHaveLength(1)
  })
})
```

- Replace the panel's `describe('macOS: a failed install gates the row …')` block with tests of `MacFinderIntegrationPanel`. Mount it with the same bindings `openPanel` uses plus `...finderSetup, ...finderSetupCopy, subscribeFinderSetup: () => () => {}` and the backend `{ finder_setup_state: () => view, finder_setup_retry: () => undefined }`. Assert: (1) `failed/timeout` → `visibleErrorSurfaces` has length 1, one `Try again`, no toast; (2) `user_disabled` → a `role="status"` element with `Open System Settings`, no error surfaces; (3) `ready` → no error surface and an `Active` chip; (4) the calls never include `install_finder_location` or `finder_location_state`. Keep the `describe('Windows: …')` block exactly as it is. It mounts `ExplorerIntegrationPanel` with `platform: 'windows'` and is the proof that Windows is unchanged.

(d) `tests/finderInstallCard.test.ts`: delete the `shouldRetryAfterUserEnabledPoll` describe (lines 103–120) and the `finderLocationButtonPlan` describe (lines 121–133). The helpers are deleted. They were macOS-only: the poll moved into the Rust driver, and the button plan into the reconciler-driven pane.

Run RED: `bun test tests/finderSetupSourceContract.test.ts tests/statusFinderSetup.test.tsx tests/finderInstallOneSurface.test.tsx > $EVID/t17-red.log 2>&1`. Expected failures: `MacFinderIntegrationPanel not found`, `macFinderSetupState not found`, the census (`finder_location_state` still in MacSettings? no, Task 16 removed it; `SettingsView.tsx` still names `install_finder_location`), and the macOS SyncFolder tests (`Install in Finder` present).

- [ ] **Step 2: Implement**

`src/finderInstallCard.ts`: delete `shouldRetryAfterUserEnabledPoll` and `finderLocationButtonPlan`, and add to the module doc: "Windows/Linux only since spec 2026-10-06: macOS surfaces use finderSetup.ts / finderSetupCopy.ts."

`src/pages/SyncFolder.tsx`:

- Imports: drop `finderLocationButtonPlan`; add `import { FINDER_ACTION_COMMAND, loadFinderSetup, runFinderSetupAction, subscribeFinderSetup, type FinderSetupView } from '../finderSetup'` and `import { FINDER_ACTION_FAILED, FINDER_STATUS_PILL, finderSetupPresentation, type FinderSetupAction } from '../finderSetupCopy'`.
- State: `const [finderView, setFinderView] = useState<FinderSetupView | null>(null)`.
- Replace the mount effect's `desktop_platform` and `finder_location_state` calls with:

```tsx
    command<DesktopPlatform>('desktop_platform').then((result) => {
      if (result.ok) setPlatform(result.value)
      if (result.ok && result.value === 'macos') {
        // Spec 2026-10-06 §10: the reconciler's state; this pane never installs on macOS.
        void loadFinderSetup().then((view) => {
          if (view.ok) setFinderView(view.value)
          else setNotice(view.unsupported ? commandUnavailableLabel('finder_setup_state') : view.reason)
        })
        return
      }
      command<FinderInstallState>('finder_location_state').then((state) => {
        if (state.ok) setInstallState(state.value)
        else setNotice(state.unsupported ? commandUnavailableLabel('finder_location_state') : state.reason)
      })
    })
```

(`sync_status` stays as it is.) Then add, after that effect:

```tsx
  // macOS: finder-setup-changed replaces re-reading (spec §10).
  useEffect(() => (platform === 'macos' ? subscribeFinderSetup(setFinderView) : undefined), [platform])
```

- Add the action handler (a failed action is a toast):

```tsx
  const runFinderAction = async (action: FinderSetupAction) => {
    setBusy(true)
    const result = await runFinderSetupAction(action)
    setBusy(false)
    if (!result.ok) {
      showToast({ variant: 'error', title: FINDER_ACTION_FAILED[action], message: result.unsupported ? commandUnavailableLabel(FINDER_ACTION_COMMAND[action]) : result.reason })
    }
  }
```

- In `resetFinderIntegration`, replace the re-read with: `if (platform === 'macos') { const view = await loadFinderSetup(); if (view.ok) setFinderView(view.value) } else { const finderState = await command<FinderInstallState>('finder_location_state'); if (finderState.ok) setInstallState(finderState.value) }`.
- Render: compute

```tsx
  const isMacos = platform === 'macos'
  const macPresentation = isMacos ? finderSetupPresentation(finderView) : null
  const installed = isMacos ? finderView?.setup === 'ready' : (installState?.installed ?? false)
  const finderNotice = isMacos ? null : finderInstallNotice(installState)
```

The page copy on macOS is `Beebeeb appears as a system-managed Finder location. Offline folders are controlled separately.`. The status pill text is `isMacos ? (finderView ? FINDER_STATUS_PILL[finderView.setup] : 'Loading') : installed ? 'Installed' : 'Needs install'`. Add the macOS notice block right after the existing non-macOS `finderNotice` blocks:

```tsx
      {macPresentation?.kind === 'notice' && (
        <div
          className={macPresentation.tone === 'alert' ? 'notice error' : 'notice'}
          role={macPresentation.tone === 'alert' ? 'alert' : 'status'}
          data-error-surface={macPresentation.tone === 'alert' ? 'finder-setup' : undefined}
          style={{ marginBottom: 14 }}
        >
          <div>{macPresentation.sentence}</div>
          <div className="button-row" style={{ marginTop: 10 }}>
            <button className="button" onClick={() => void runFinderAction(macPresentation.action)} disabled={busy}>
              {macPresentation.actionLabel}
            </button>
          </div>
        </div>
      )}
      {macPresentation?.kind === 'adding' && (
        <div className="notice" role="status" style={{ marginBottom: 14 }}>{macPresentation.line}</div>
      )}
```

The button row becomes: `Choose location` only `!isMacos`; `Install in Finder` only `!isMacos`; `Open in Finder` when `!isMacos || installed`. The macOS location text is `'Beebeeb in Finder'`. Delete `openSystemSettings` and the `finderNotice?.kind === 'user_disabled'` block's System Settings button only if `bunx tsc`/eslint report them unused. The non-macOS user-disabled branch can no longer occur, but leaving it changes nothing on Linux. Be conservative: keep it.

**Rule for both pages:** every identifier imported from `finderSetup`/`finderSetupCopy` is evaluated only inside `isMacos` branches. `tests/syncRoot.test.ts` mounts these pages with `platform: 'windows'` and binds none of them.

`src/pages/Status.tsx`:

- Imports: `DesktopPlatform` from `../desktopApi`; `import { loadFinderSetup, subscribeFinderSetup, type FinderSetupView } from '../finderSetup'`; `import { FINDER_ADDING_LINE, FINDER_STATUS_PILL, finderSetupPresentation } from '../finderSetupCopy'`.
- Add next to `finderSetupState`:

```tsx
/** macOS (spec 2026-10-06): the reconciler's state, in this page's label/className/detail shape. */
function macFinderSetupState(view: FinderSetupView | null) {
  if (!view) return { label: 'Loading', className: '', detail: 'Checking Finder setup.' }
  const presentation = finderSetupPresentation(view)
  const label = FINDER_STATUS_PILL[view.setup]
  if (presentation.kind === 'notice') return { label, className: presentation.tone === 'alert' ? 'error' : 'warn', detail: presentation.sentence }
  if (presentation.kind === 'ready') return { label, className: 'ok', detail: presentation.line }
  return { label, className: 'warn', detail: presentation.kind === 'adding' ? presentation.line : FINDER_ADDING_LINE }
}
```

- In `Status`, add `const [platform, setPlatform] = useState<DesktopPlatform | null>(null)` and `const [finderView, setFinderView] = useState<FinderSetupView | null>(null)`, add a mount effect `command<DesktopPlatform>('desktop_platform').then((r) => setPlatform(r.ok ? r.value : 'unknown'))`, and replace the 3-second effect with:

```tsx
  useEffect(() => {
    if (platform === null) return
    const mac = platform === 'macos'
    let cancelled = false
    const refresh = async () => {
      const [next, finderState] = await Promise.all([
        loadSyncStatus(),
        mac ? Promise.resolve(null) : command<FinderInstallState>('finder_location_state'),
      ])
      if (cancelled) return
      setStatus(next)
      if (finderState === null) return
      if (finderState.ok) {
        setFinderInstallState(finderState.value)
        setFinderNotice(null)
      } else {
        setFinderInstallState(null)
        setFinderNotice(finderState.unsupported ? commandUnavailableLabel('finder_location_state') : finderState.reason)
      }
    }
    void refresh()
    const id = window.setInterval(refresh, 3000)
    // macOS: finder-setup-changed instead of polling the Finder state (spec §10).
    let unsubscribe = () => {}
    if (mac) {
      void loadFinderSetup().then((result) => {
        if (!cancelled && result.ok) setFinderView(result.value)
      })
      unsubscribe = subscribeFinderSetup((view) => {
        if (!cancelled) setFinderView(view)
      })
    }
    return () => {
      cancelled = true
      window.clearInterval(id)
      unsubscribe()
    }
  }, [platform])
```

- `const finderSetup = platform === 'macos' ? macFinderSetupState(finderView) : finderSetupState(finderInstallState)`.

`src/windows/views/SettingsView.tsx`:

- Delete the macOS line from `shellIntegrationCommandsFor`, and update its doc: "macOS never reaches this panel (`ShellOrFinderPanel` sends it to `MacFinderIntegrationPanel`)".
- In `ExplorerIntegrationPanel`, delete the macOS-only parts: the `inline` branch in `enable` (it becomes `const r = await command<ShellIntegrationState>(commands.install)`, followed by the existing non-inline handling), `openSystemSettings`, `const notice = …`, and the two notice blocks (`notice?.kind === 'error'` / `'user_disabled'`). Remove `finderInstallNotice, finderInstallStateAfterAttempt, finderInstallStateWhileAttempting` from the import on line 57.
- Add:

```tsx
/** Dispatches the main window's integration panel: macOS reads the reconciler (spec 2026-10-06). */
function ShellOrFinderPanel({ status }: { status: SyncStatus | null }) {
  const { name, resolved } = usePlatform()
  return resolved && name === 'macos' ? <MacFinderIntegrationPanel /> : <ExplorerIntegrationPanel status={status} />
}

/** macOS: the reconciler's state, read-only. No install action exists here (R5). */
function MacFinderIntegrationPanel() {
  const regionLabel = useRegionLabel()
  const { showToast } = useToast()
  const [view, setView] = useState<FinderSetupView | null>(null)
  useEffect(() => {
    let cancelled = false
    void loadFinderSetup().then((result) => {
      if (!cancelled && result.ok) setView(result.value)
    })
    const unsubscribe = subscribeFinderSetup((next) => {
      if (!cancelled) setView(next)
    })
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [])
  const presentation = finderSetupPresentation(view)
  // A one-off action that gates nothing: a failure is a toast.
  const act = async (action: FinderSetupAction) => {
    const result = await runFinderSetupAction(action)
    if (!result.ok) {
      showToast({ variant: 'error', title: FINDER_ACTION_FAILED[action], message: result.unsupported ? commandUnavailableLabel(FINDER_ACTION_COMMAND[action]) : result.reason })
    }
  }
  const alert = presentation.kind === 'notice' && presentation.tone === 'alert'
  return (
    <SettingsSectionShell>
      <PageHeader title="Finder integration" subtitle={`Beebeeb appears in Finder as a sync folder. Files are encrypted on this Mac before they leave. ${regionLabel}.`} />
      <Card style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16, padding: '14px 16px', background: T.paper2 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: T.ink, marginBottom: 3 }}>{FINDER_SETUP_TITLE}</div>
          <div style={{ fontSize: 11.5, color: T.ink3, lineHeight: 1.5 }}>
            {presentation.kind === 'notice' ? FINDER_STATUS_PILL[view?.setup ?? 'missing'] : presentation.line || 'Checking...'}
          </div>
        </div>
        {presentation.kind === 'ready' && <Chip tone="green">Active</Chip>}
      </Card>
      {presentation.kind === 'notice' && (
        <div
          role={alert ? 'alert' : 'status'}
          data-error-surface={alert ? 'finder-setup' : undefined}
          style={{
            marginTop: 10,
            padding: '10px 12px',
            background: alert ? 'var(--err-bg)' : T.paper2,
            border: alert ? '1px solid var(--err-line)' : `1px solid ${T.line2}`,
            borderRadius: 8,
            color: T.ink,
            fontSize: 12,
            lineHeight: 1.5,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
          }}
        >
          <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>{presentation.sentence}</span>
          <PrimaryBtn onClick={() => void act(presentation.action)}>{presentation.actionLabel}</PrimaryBtn>
        </div>
      )}
    </SettingsSectionShell>
  )
}
```

  Imports to add: `import { FINDER_ACTION_COMMAND, loadFinderSetup, runFinderSetupAction, subscribeFinderSetup, type FinderSetupView } from '../../finderSetup'` and `import { FINDER_ACTION_FAILED, FINDER_SETUP_TITLE, FINDER_STATUS_PILL, finderSetupPresentation, type FinderSetupAction } from '../../finderSetupCopy'`.
- Replace `return <ExplorerIntegrationPanel status={status} />` (line ~2015) with `return <ShellOrFinderPanel status={status} />`.

- [ ] **Step 3: Run GREEN, then the whole frontend gate**

```bash
cd $WT && bun test tests/finderSetupSourceContract.test.ts tests/statusFinderSetup.test.tsx tests/finderInstallOneSurface.test.tsx tests/finderInstallCard.test.ts tests/syncRoot.test.ts > $EVID/t17-green.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t17-green.log
bun test > $EVID/t17-bun-test.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)|Ran [0-9]+ tests" $EVID/t17-bun-test.log
bunx tsc --noEmit -p . > $EVID/t17-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/t17-eslint.log 2>&1; echo "eslint rc=$?"
```

Expected: `0 fail` everywhere. `tests/syncRoot.test.ts` keeps its baseline pass count (Task 1 step 0): this is the Windows proof for both pages. tsc and eslint give `rc=0`. Paste the total `N pass / 0 fail` line next to the baseline. If eslint's `no-ad-hoc-error-surface` reports a new straggler, the fix is the house rule: a failed *action* is a toast, a LOAD failure is inline. Never add a disable comment without a reason that names the gated control.

- [ ] **Step 4: Mutation check**

Add `void command('install_finder_location')` as the first line of `MacFinderStep`'s mount effect. Expected: `macOS-only units never name them` fails, naming `Onboarding.tsx#MacFinderStep`, and the onboarding test `… none of the install-era commands is called` fails. Remove the line.

- [ ] **Step 5: Commit**

```bash
cd $WT && git add tests/statusFinderSetup.test.tsx tests/finderSetupSourceContract.test.ts
git commit -m "macOS surfaces: SyncFolder, Status and the main-window panel follow the reconciler; no macOS path installs (spec A §10, §13.1)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src/pages/SyncFolder.tsx src/pages/Status.tsx src/windows/views/SettingsView.tsx src/finderInstallCard.ts tests/finderInstallOneSurface.test.tsx tests/finderInstallCard.test.ts tests/statusFinderSetup.test.tsx tests/finderSetupSourceContract.test.ts
git show --stat HEAD
```

---

## Task 18: Re-sign-in in place: the banner, the sign-in step and the switch warning (R8, frontend)

**Lane T.** The frontend half of R8 (backend: Tasks 11, 12). On macOS, "Sign in again" (the persistent `AuthExpiredBanner` and VersionCenter's review action, both through `forceReauth`) no longer calls `clearSession`. It opens sign-in in place (`open_reauth_window`). A same-account sign-in closes the window and sync resumes. A different account gets the warning drawn in Task 13. **Windows and Linux keep today's flow** (clear first, then onboarding), and the tests prove it.

**Files:**
- Modify: `src/desktopApi.ts` (`DesktopLoginResult`, `desktopLogin2fa`'s return type, `ForceReauthApi`, `forceReauth`)
- Modify: `src/onboardingSignIn.ts` (results carry what the sign-in became)
- Create: `src/accountSwitchCopy.ts`
- Modify: `src/Onboarding.tsx` (`mode` prop, `SignInStep` reports the outcome, `OnboardingView` routes it, new `AccountSwitchStep`)
- Modify: `src/main.tsx` (pass `mode`)
- Modify: `tests/forceReauth.test.ts`, `tests/onboardingSignIn.test.ts`
- Create: `tests/reauthInPlace.test.tsx`

**Interfaces:**
- Consumes: the Rust `LoginOutcome` JSON (Task 11) and the `open_reauth_window` command (Task 11); the warning copy drawn in Task 13.
- Produces:
  - `DesktopLoginResult { requires_2fa: boolean; reauthenticated?: boolean; vault_unlocked?: boolean; account_mismatch?: { pending_changes: number } | null }`; `desktopLogin2fa(code): Promise<CommandResult<DesktopLoginResult>>`.
  - `type SignInSettled = { kind: 'fresh' } | { kind: 'reauthenticated'; vaultUnlocked: boolean } | { kind: 'account_mismatch'; pendingChanges: number }`; `settledFrom(value: DesktopLoginResult | null | undefined): SignInSettled`.
  - `PasswordStepResult` ok branch → `{ ok: true; requiresTotp: boolean; settled: SignInSettled }`; `TotpStepResult` ok branch → `{ ok: true; settled: SignInSettled }`.
  - `ForceReauthApi { platform, clearSession, openOnboardingWindow, openReauthWindow }`.
  - `accountSwitchCopy.ts`: `ACCOUNT_SWITCH_TITLE`, `accountSwitchBody(pendingChanges: number)`, `ACCOUNT_SWITCH_CONFIRM`, `ACCOUNT_SWITCH_CANCEL`, `ACCOUNT_SWITCH_FAILED`.
  - `Onboarding({ mode }: { mode?: 'setup' | 'reauth' })`, `function AccountSwitchStep({ pendingChanges, onSwitched, onCancel })`.

- [ ] **Step 1: Tests first**

`tests/forceReauth.test.ts`: add `platform: async () => ({ ok: true, value: 'windows' })` and `openReauthWindow: async () => ({ ok: true, value: undefined })` to `fakeApi`'s defaults, so the two existing tests keep testing the unchanged Windows/Linux path. Then add:

```ts
describe('forceReauth on macOS (R8: re-sign-in in place)', () => {
  test('opens sign-in in place and never clears the session', async () => {
    const calls: string[] = []
    const api = fakeApi({
      platform: async () => ({ ok: true, value: 'macos' }),
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: undefined } },
      openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
      openReauthWindow: async () => { calls.push('open_reauth_window'); return { ok: true, value: undefined } },
    })
    expect((await forceReauth(api)).ok).toBe(true)
    expect(calls).toEqual(['open_reauth_window'])
  })

  test('if the platform cannot be read, nothing is cleared', async () => {
    const calls: string[] = []
    const api = fakeApi({
      platform: async () => ({ ok: false, reason: 'ipc down', unsupported: false }),
      clearSession: async () => { calls.push('clear_session'); return { ok: true, value: undefined } },
    })
    expect((await forceReauth(api)).ok).toBe(false)
    expect(calls).toEqual([])
  })

  for (const platform of ['windows', 'linux'] as const) {
    test(`${platform} is unchanged: clear first, then onboarding`, async () => {
      const calls: string[] = []
      const api = fakeApi({
        platform: async () => ({ ok: true, value: platform }),
        clearSession: async () => { calls.push('clear_session'); return { ok: true, value: undefined } },
        openOnboardingWindow: async () => { calls.push('open_onboarding_window'); return { ok: true, value: undefined } },
        openReauthWindow: async () => { calls.push('open_reauth_window'); return { ok: true, value: undefined } },
      })
      await forceReauth(api)
      expect(calls).toEqual(['clear_session', 'open_onboarding_window'])
    })
  }
})
```

`tests/onboardingSignIn.test.ts`: update every expected `{ ok: true, requiresTotp: … }` to include `settled: { kind: 'fresh' }` (the fake `desktopLogin` answers `{ requires_2fa: … }` only), and every `{ ok: true }` from `submitTotpCode` to `{ ok: true, settled: { kind: 'fresh' } }`. Then add:

```ts
describe('settledFrom (R8)', () => {
  test('maps the three outcomes', () => {
    expect(settledFrom({ requires_2fa: false })).toEqual({ kind: 'fresh' })
    expect(settledFrom(null)).toEqual({ kind: 'fresh' })
    expect(settledFrom({ requires_2fa: false, reauthenticated: true, vault_unlocked: true })).toEqual({ kind: 'reauthenticated', vaultUnlocked: true })
    expect(settledFrom({ requires_2fa: false, reauthenticated: true, vault_unlocked: false })).toEqual({ kind: 'reauthenticated', vaultUnlocked: false })
    expect(settledFrom({ requires_2fa: false, account_mismatch: { pending_changes: 3 } })).toEqual({ kind: 'account_mismatch', pendingChanges: 3 })
  })
})
```

`tests/reauthInPlace.test.tsx`:

```tsx
/**
 * R8 (spec 2026-10-06): "Sign in again as the same account and only the session token is replaced.
 * … Signing in as a different account is an account switch: full sign-out, with a warning if edits
 * haven't uploaded yet." The onboarding window in reauth mode, the routing of the three outcomes,
 * and the switch warning. The copy must match the design artefact (Task 13).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as switchCopy from '../src/accountSwitchCopy'
import { loadComponent, mount, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const Card = loadComponent('Onboarding.tsx', 'Card', { React })
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

/** Stand-ins for the steps OnboardingView routes between. The harness does not expand components,
 *  so a rendered step is an element whose `type` is one of these functions. */
function stepStubs() {
  const byName: Record<string, (props: any) => null> = {}
  for (const name of ['SignInStep', 'UnlockStep', 'MacFinderStep', 'FinderInstallStep', 'PinningStep', 'ReadyStep', 'AccountSwitchStep']) {
    byName[name] = () => null
  }
  return {
    byName,
    bindings: { ...byName, STEPS: [], Wordmark: () => null, useRegionLabel: () => 'Stored in the EU' },
  }
}

function openView(mode: 'setup' | 'reauth') {
  const closed: string[] = []
  const stubs = stepStubs()
  const m = mount('Onboarding.tsx', 'OnboardingView', {
    backend: {
      sync_status: () => ({ logged_in: true, vault_unlocked: true, engine: 'running', sync_root: '/x', syncing: 0, cloud_only: 0, conflicts: 0 }),
      desktop_platform: () => 'macos',
      finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
    },
    props: { mode },
    bindings: {
      ...desktopApi,
      loadFinderSetup: () => desktopApi.command('finder_setup_state'),
      getCurrentWindow: () => ({ close: async () => { closed.push('close') } }),
      ...stubs.bindings,
    },
  })
  mounted.push(m)
  const find = (name: string) => m.elements().find((el) => el.type === stubs.byName[name])
  return { m, find, has: (name: string) => find(name) !== undefined, closed }
}

describe('reauth mode', () => {
  test('starts at sign-in even when sync_status says signed in and unlocked (setup mode still fast-forwards)', async () => {
    const reauth = openView('reauth')
    await reauth.m.flush(); await reauth.m.flush()
    expect(reauth.has('SignInStep')).toBe(true)
    const setup = openView('setup')
    await setup.m.flush(); await setup.m.flush()
    expect(setup.has('ReadyStep')).toBe(true)
  })

  test('the same account with its keys here closes the window and never signs out', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: true })
    await v.m.flush()
    expect(v.closed).toEqual(['close'])
    expect(v.m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('the same account without keys on this Mac goes on to the recovery phrase', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'reauthenticated', vaultUnlocked: false })
    await v.m.flush()
    expect(v.has('UnlockStep')).toBe(true)
  })

  test('another account shows the switch warning with the pending count, and nothing is cleared yet', async () => {
    const v = openView('reauth')
    await v.m.flush(); await v.m.flush()
    v.find('SignInStep')!.props.onDone({ kind: 'account_mismatch', pendingChanges: 3 })
    await v.m.flush()
    expect(v.find('AccountSwitchStep')!.props.pendingChanges).toBe(3)
    expect(v.m.calls.map((c) => c.name)).not.toContain('clear_session')
  })
})

describe('AccountSwitchStep', () => {
  function openSwitch(pendingChanges: number, clear: () => unknown = () => undefined) {
    const events: string[] = []
    const m = mount('Onboarding.tsx', 'AccountSwitchStep', {
      backend: { clear_session: clear },
      props: { pendingChanges, onSwitched: () => events.push('switched'), onCancel: () => events.push('cancelled') },
      bindings: { ...desktopApi, ...switchCopy, Card },
    })
    mounted.push(m)
    return { m, events }
  }

  test('says how many changes would be removed, in the drawn copy', async () => {
    const { m } = openSwitch(3)
    await m.flush()
    // The root is the (unexpanded) Card element: its title and copy are props.
    expect(m.tree().props.title).toBe(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(m.tree().props.copy).toBe(switchCopy.accountSwitchBody(3))
  })

  test('Cancel changes nothing', async () => {
    const { m, events } = openSwitch(3)
    await m.flush()
    await m.click('Cancel')
    expect(events).toEqual(['cancelled'])
    expect(m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('Sign out and switch signs out once, then hands back to sign-in', async () => {
    const { m, events } = openSwitch(3)
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(1)
    expect(events).toEqual(['switched'])
  })

  test('a failed sign-out is a toast, and the switch does not happen', async () => {
    const { m, events } = openSwitch(1, () => { throw new Error('engine busy') })
    await m.flush()
    await m.click('Sign out and switch')
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    expect(events).toEqual([])
  })
})

describe('accountSwitchCopy', () => {
  test('the exact strings, singular and plural, and the artefact draws them (design first)', () => {
    expect(switchCopy.ACCOUNT_SWITCH_TITLE).toBe('Switch to a different account?')
    expect(switchCopy.accountSwitchBody(0)).toBe('This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.')
    expect(switchCopy.accountSwitchBody(1)).toBe('This Mac is signed in to another Beebeeb account, and 1 change on this Mac hasn’t uploaded yet. Switching signs that account out of this Mac and removes it.')
    expect(switchCopy.accountSwitchBody(3)).toBe('This Mac is signed in to another Beebeeb account, and 3 changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.')
    expect([switchCopy.ACCOUNT_SWITCH_CANCEL, switchCopy.ACCOUNT_SWITCH_CONFIRM]).toEqual(['Cancel', 'Sign out and switch'])
    const html = readFileSync(new URL('../design/hifi/macos-settings-dialogs.html', import.meta.url), 'utf8')
    for (const text of [switchCopy.ACCOUNT_SWITCH_TITLE, switchCopy.accountSwitchBody(3), switchCopy.accountSwitchBody(0), switchCopy.ACCOUNT_SWITCH_CONFIRM]) {
      expect(html).toContain(text)
    }
  })
})
```

RED:

```bash
cd $WT && bun test tests/forceReauth.test.ts tests/onboardingSignIn.test.ts tests/reauthInPlace.test.tsx > $EVID/t18-red.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)|Cannot find|Missing component" $EVID/t18-red.log | head
```

Expected: failures. `Cannot find module '../src/accountSwitchCopy'`; the macOS `forceReauth` tests fail (`clear_session` is called); `settledFrom` is missing. Paste them into Notes.

- [ ] **Step 2: Implement**

`src/accountSwitchCopy.ts`:

```ts
/**
 * R8 (spec 2026-10-06): the one warning an account switch shows. Drawn first in
 * design/hifi/macos-settings-dialogs.html (Task 13); tests/reauthInPlace.test.tsx holds both to the
 * same strings. Honest about the loss: the changes that have not uploaded are removed.
 */
export const ACCOUNT_SWITCH_TITLE = 'Switch to a different account?'
export const ACCOUNT_SWITCH_CONFIRM = 'Sign out and switch'
export const ACCOUNT_SWITCH_CANCEL = 'Cancel'
export const ACCOUNT_SWITCH_FAILED = 'Couldn’t sign out'

export function accountSwitchBody(pendingChanges: number): string {
  if (pendingChanges <= 0) {
    return 'This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.'
  }
  if (pendingChanges === 1) {
    return 'This Mac is signed in to another Beebeeb account, and 1 change on this Mac hasn’t uploaded yet. Switching signs that account out of this Mac and removes it.'
  }
  return `This Mac is signed in to another Beebeeb account, and ${pendingChanges} changes on this Mac haven’t uploaded yet. Switching signs that account out of this Mac and removes them.`
}
```

`src/desktopApi.ts`:

```ts
/** Shape returned by `desktop_login` and `desktop_login_2fa`. R8 adds the last three fields. */
export interface DesktopLoginResult {
  requires_2fa: boolean
  /** The account on this Mac signed in again in place; nothing was cleared. */
  reauthenticated?: boolean
  /** With `reauthenticated`: the keys are here, so no recovery phrase is needed. */
  vault_unlocked?: boolean
  /** Another account signed in; nothing changed on this Mac. */
  account_mismatch?: { pending_changes: number } | null
}
```

`desktopLogin2fa` becomes `Promise<CommandResult<DesktopLoginResult>>` → `command<DesktopLoginResult>('desktop_login_2fa', { code })`. Then:

```ts
export interface ForceReauthApi {
  platform: () => Promise<CommandResult<DesktopPlatform>>
  clearSession: typeof clearSession
  openOnboardingWindow: () => Promise<CommandResult<void>>
  openReauthWindow: () => Promise<CommandResult<void>>
}

const defaultForceReauthApi: ForceReauthApi = {
  platform: () => command<DesktopPlatform>('desktop_platform'),
  clearSession,
  openOnboardingWindow: () => command<void>('open_onboarding_window'),
  openReauthWindow: () => command<void>('open_reauth_window'),
}

/**
 * "Sign in again". On macOS (ruling R8, spec 2026-10-06) it opens sign-in IN PLACE: nothing is
 * cleared, so a same-account sign-in keeps Finder, keys, cache and pending edits, and another
 * account gets the switch warning. On Windows and Linux it is unchanged (task 1546 Codex round 2,
 * finding 2): clear the session first, then open onboarding, which otherwise fast-forwards an
 * "unlocked, configured" user past the sign-in form. If the platform cannot be read, nothing is
 * cleared: clearing on a Mac by mistake is the data loss R8 fixes.
 */
export async function forceReauth(api: ForceReauthApi = defaultForceReauthApi): Promise<CommandResult<void>> {
  const platform = await api.platform()
  if (!platform.ok) return platform
  if (platform.value === 'macos') return api.openReauthWindow()
  const cleared = await api.clearSession()
  if (!cleared.ok) return cleared
  return api.openOnboardingWindow()
}
```

`src/onboardingSignIn.ts`:

```ts
import { desktopLogin, desktopLogin2fa, commandUnavailableLabel, type DesktopLoginResult } from './desktopApi'

/** What a completed sign-in became (R8). */
export type SignInSettled =
  | { kind: 'fresh' }
  | { kind: 'reauthenticated'; vaultUnlocked: boolean }
  | { kind: 'account_mismatch'; pendingChanges: number }

export function settledFrom(value: DesktopLoginResult | null | undefined): SignInSettled {
  if (value?.account_mismatch) return { kind: 'account_mismatch', pendingChanges: value.account_mismatch.pending_changes }
  if (value?.reauthenticated) return { kind: 'reauthenticated', vaultUnlocked: Boolean(value.vault_unlocked) }
  return { kind: 'fresh' }
}
```

The types become `{ ok: true; requiresTotp: boolean; settled: SignInSettled }` and `{ ok: true; settled: SignInSettled }`. `submitPassword` returns `{ ok: true, requiresTotp: result.value.requires_2fa, settled: settledFrom(result.value) }`, and `submitTotpCode` returns `{ ok: true, settled: settledFrom(result.value) }`.

`src/main.tsx`: in `HostOnboarding`, replace `<Onboarding />` with `<Onboarding mode={params.get('mode') === 'reauth' ? 'reauth' : 'setup'} />`.

`src/Onboarding.tsx`:

1. Imports: `import { type SignInSettled } from './onboardingSignIn'` (next to the existing import) and `import { ACCOUNT_SWITCH_CANCEL, ACCOUNT_SWITCH_CONFIRM, ACCOUNT_SWITCH_FAILED, ACCOUNT_SWITCH_TITLE, accountSwitchBody } from './accountSwitchCopy'`.
2. `type Step = 'signin' | 'unlock' | 'finder' | 'pinning' | 'ready' | 'switch'`.
3. `export default function Onboarding({ mode = 'setup' }: { mode?: 'setup' | 'reauth' }) { return <OnboardingErrorBoundary><OnboardingView mode={mode} /></OnboardingErrorBoundary> }`.
4. `function OnboardingView({ mode }: { mode: 'setup' | 'reauth' })`. Add `const [pendingSwitch, setPendingSwitch] = useState(0)`. In the mount effect, directly after `if (platformResult.ok) setPlatform(platformResult.value)`, add:

```tsx
      // R8: "Sign in again" opens this window in reauth mode. It starts at sign-in whatever
      // sync_status says; nothing was cleared, so the status still reads signed in and unlocked.
      if (mode === 'reauth') return
```

  Then add the router and use it:

```tsx
  const afterSignIn = (settled: SignInSettled) => {
    if (settled.kind === 'account_mismatch') {
      setPendingSwitch(settled.pendingChanges)
      setStep('switch')
      return
    }
    if (settled.kind === 'reauthenticated' && settled.vaultUnlocked) {
      // The same account, its keys here: sync resumes; nothing else to set up.
      void getCurrentWindow().close()
      return
    }
    setStep('unlock')
  }
```

  `{step === 'signin' && <SignInStep onDone={afterSignIn} />}`, plus:

```tsx
        {step === 'switch' && (
          <AccountSwitchStep
            pendingChanges={pendingSwitch}
            onSwitched={() => setStep('signin')}
            onCancel={() => (mode === 'reauth' ? void getCurrentWindow().close() : setStep('signin'))}
          />
        )}
```

  The `useEffect`'s dependency list becomes `[mode]`.
5. `SignInStep({ onDone }: { onDone: (settled: SignInSettled) => void })`: in `submitPasswordForm`, `onDone()` becomes `onDone(result.settled)`. In `submitTotpForm`, `onDone()` becomes `onDone(result.settled)`.
6. Add above `UnlockStep`:

```tsx
/**
 * R8: another account is signing in on this Mac. Nothing has changed yet. "Sign out and switch"
 * is the full sign-out (Finder entry removed, queue and cache purged), then a fresh sign-in.
 * A failed sign-out is a toast (an action that gates nothing more than itself).
 */
function AccountSwitchStep({ pendingChanges, onSwitched, onCancel }: { pendingChanges: number; onSwitched: () => void; onCancel: () => void }) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)
  const switchAccount = async () => {
    setBusy(true)
    const result = await command<void>('clear_session')
    setBusy(false)
    if (!result.ok) {
      showToast({ variant: 'error', title: ACCOUNT_SWITCH_FAILED, message: result.unsupported ? commandUnavailableLabel('clear_session') : result.reason })
      return
    }
    onSwitched()
  }
  return (
    <Card title={ACCOUNT_SWITCH_TITLE} copy={accountSwitchBody(pendingChanges)}>
      <div className="button-row" style={{ marginTop: 16 }}>
        <button className="button" onClick={onCancel} disabled={busy}>
          {ACCOUNT_SWITCH_CANCEL}
        </button>
        <button className="button danger" onClick={() => void switchAccount()} disabled={busy}>
          {ACCOUNT_SWITCH_CONFIRM}
        </button>
      </div>
    </Card>
  )
}
```

  (`.button.danger` exists in `src/design.css:935`.)

- [ ] **Step 3: GREEN + typecheck + lint + neighbours**

```bash
cd $WT && bun test tests/forceReauth.test.ts tests/onboardingSignIn.test.ts tests/reauthInPlace.test.tsx tests/onboardingFinderStep.test.tsx tests/conflictTextnessAndAuthBanner.test.ts > $EVID/t18-green.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t18-green.log
bunx tsc --noEmit -p . > $EVID/t18-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/t18-eslint.log 2>&1; echo "eslint rc=$?"
```

Expected: `0 fail`, and tsc and eslint give `rc=0`. `tests/conflictTextnessAndAuthBanner.test.ts` (it covers the banner) keeps its count. `WindowsFirstRun.tsx` is untouched. If `tsc` reports that it reads `desktopLogin2fa`'s value as `void`, the change is type-only: keep its behaviour and note it.

- [ ] **Step 4: Mutation checks**

1. In `forceReauth`, swap the order so `clearSession` runs before the platform branch. Expected: `opens sign-in in place and never clears the session` fails. Restore it.
2. In `OnboardingView`, delete `if (mode === 'reauth') return`. Expected: `starts at sign-in even when sync_status says …` fails. Restore it.
3. In `AccountSwitchStep`, call `onSwitched()` before checking `result.ok`. Expected: `a failed sign-out is a toast, and the switch does not happen` fails. Restore it.

- [ ] **Step 5: Commit**

```bash
cd $WT && git add src/accountSwitchCopy.ts tests/reauthInPlace.test.tsx
git commit -m "sign in again (R8, macOS): in place, never clears; another account gets the switch warning; Windows/Linux unchanged

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- src/desktopApi.ts src/onboardingSignIn.ts src/accountSwitchCopy.ts src/Onboarding.tsx src/main.tsx tests/forceReauth.test.ts tests/onboardingSignIn.test.ts tests/reauthInPlace.test.tsx
git show --stat HEAD
```

---

## Task 19: Docs that change with the code (spec §14)

**Lane T, last commit before the PR.** `RELEASE_NOTES.md` is written at release time (Task 21 step 9), because the release script refuses notes without counted test results, and the version is not known yet.

**Files:**
- Modify: `CLAUDE.md` ("Current macOS integration state", after its "Current release identifiers" list)
- Modify: `docs/CAPABILITIES.md` (the command table rows at lines ~266–270, plus the "Finder location" bullet at ~211)

- [ ] **Step 1: `CLAUDE.md`**

Insert after the bullet list of release identifiers:

```markdown
### Finder setup reconciler (spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md`)

On macOS nobody installs the Finder location any more. One Rust task in `src-tauri/src/finder_setup/`
keeps Beebeeb in Finder: a pure `core` (the state machine of spec §5), a pure `policy`
(classification by NSError domain + code only; retries at 0/5/15/45 s), a pure `launch_location`,
and one `driver` task that runs one `NSFileProviderManager` call at a time through
`macos/FileProviderBridge.m`, which returns structured `(domain, code, message, underlying)` errors.
Triggers: launch (after the startup restore), keys arriving (`apply_session`,
`desktop_unlock_with_recovery_phrase`, `unlock_vault`), "Try again", and the userEnabled flip.
Sign-out removes the domain and holds until the next sign-in. Repair removes, then checks once.
Lock cancels a running check. Surfaces read `finder_setup_state` and listen to
`finder-setup-changed`. All copy lives in `src/finderSetupCopy.ts`.

**Lifecycle log.** From the app's point of view it is `~/Library/Logs/Beebeeb/lifecycle.log`. The app is
sandboxed, so on disk it is `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log`:
1 MB × 3 files, typed events, redacted NSError text, and **not** a `tracing` sink. "Copy details" and
the support bundle include its last 200 lines.

**Correction — 1698.** The `DOMAIN_IDENTIFIER` comment in `macos_file_provider.rs` claimed the zombie
`io.beebeeb.desktop.FileProvider` domain owned `Beebeeb-Drive` and that a signed context could sweep it.
Both claims were wrong. Its folder is `Beebeeb-Beebeeb`, `getDomains` returns only the calling
provider's domains, and renaming our domain to "Beebeeb" made it collide with that folder. A dev Mac
that ran a build from before 18 May clears it once with the helper in spec §11.

### Re-sign-in in place (ruling R8)

"Sign in again" on macOS opens sign-in in place (`open_reauth_window`); nothing is cleared first.
`desktop_login`, `desktop_login_2fa` and the browser handoff compare the account signing in with
the recorded owner of the local data (`src-tauri/src/reauth.rs`, R10). The same account swaps only its session token:
keys, cache, queue and Finder stay, and the queue uploads afterwards. Another account gets the
switch warning (with the pending-change count), then a full sign-out, then a fresh sign-in. A
startup 401 drops only the revoked token on macOS/Linux. Windows keeps today's flow.

### Local data belongs to one account (R10)

`state.db` records the account that owns this computer's local data (`owner_user_id`, `owner_email`
in `sync_state`). Every engine start goes through `spawn_bound_engine`, which runs
`account_binding::bind_before_engine_start` first: the same account keeps everything; another
account's data, or data with no recorded owner, is reset first (Windows refuses instead); an owner
that cannot be compared stops the start. A failed reset or sign-out purge stops. Keep every engine
start behind `spawn_bound_engine`: `every_engine_start_is_bound_first` counts them.
```

- [ ] **Step 2: `docs/CAPABILITIES.md`**

Replace the rows for `finder_location_state`, `install_finder_location`, `continue_without_finder_location`, `finder_domain_user_enabled` and `open_login_items_and_extensions_settings` with the following rows, and insert the five new rows after them:

```markdown
| `finder_location_state` | `Onboarding.tsx` (Windows/Linux step), `pages/SyncFolder.tsx`, `pages/Status.tsx` (non-macOS branches) | Not called on macOS since spec 2026-10-06; on macOS it returns an error naming `finder_setup_state` |
| `install_finder_location` | `Onboarding.tsx` (Windows/Linux step), `pages/SyncFolder.tsx` (non-macOS branch) | Not called on macOS (ruling R5); on macOS it returns an error |
| `continue_without_finder_location` | `Onboarding.tsx` (Windows/Linux step) | Windows/Linux escape hatch |
| `finder_domain_user_enabled` | none | Registered, unused: the poll moved into the Rust reconciler |
| `open_login_items_and_extensions_settings` | `finderSetup.ts` | macOS "Open System Settings" for `user_disabled` |
| `finder_setup_state` | `finderSetup.ts` (Onboarding, MacSettings, SyncFolder, Status, SettingsView on macOS) | macOS reconciler state; `finder-setup-changed` carries the same view |
| `finder_setup_retry` | `finderSetup.ts` | macOS "Try again", only inside a failure |
| `finder_setup_copy_details` | `finderSetup.ts` | macOS "Copy details" (no paths, names or email) |
| `finder_setup_show_app` | `finderSetup.ts` | macOS "Show in Finder" for `not_in_applications` |
| `open_reauth_window` | `desktopApi.ts` (`forceReauth`, macOS) | R8: opens sign-in in place; clears nothing |
```

Change the `desktop_login` and `desktop_login_2fa` rows' last cell to: `Returns LoginOutcome (R8): requires_2fa, reauthenticated, vault_unlocked, account_mismatch { pending_changes }; Windows keeps refusing while signed in`.

After the "Finder location: pick root, install, open, reset/cancel …" bullet, append: `On macOS (spec 2026-10-06) the pane has no install action: it reads finder_setup_state, follows finder-setup-changed, and offers only the one action of the current failure.`

- [ ] **Step 3: Commit**

```bash
cd $WT && git commit -m "docs: the macOS Finder reconciler, the lifecycle log path, re-sign-in in place (R8), the account binding (R10), the 1698 correction (spec A §14)

Co-Authored-By: <model that wrote this> <noreply@anthropic.com>" -- CLAUDE.md docs/CAPABILITIES.md
git show --stat HEAD
```

- [ ] **Step 4: Open Lane T's PR** (lead)

It is opened after Lane R's PR has merged and Lane T has been rebased onto `origin/main` with its gate rerun (Task 21 step 2). The PR description says D0–D7 ran on the integration build, and links the evidence directory.

---

## Task 20: Clear the orphan with a one-off helper (spec §11, dev Macs only, R6)

**Lead + Guus. Not product code**: everything lives in `$SCRATCH` and is deleted afterwards. Nothing here is committed to the desktop repo. It runs **after D0** (Task 21 step 5) has captured the real `folder_taken` error on this Mac.

- [ ] **Step 1 (Guus): two Apple Development provisioning profiles**

In the developer portal (team `R8352WDJJR`): App ID `io.beebeeb.desktop` and App ID `io.beebeeb.desktop.FileProvider`, both with the App Groups capability and the group `R8352WDJJR.io.beebeeb.desktop.fileprovider`. If May's App IDs are gone, recreate them (spec §15). Then create one macOS Development profile for each, including this Mac, and download both. The lead records their file paths and UUIDs in Notes:

```bash
for p in ~/Downloads/*.provisionprofile; do echo "$p $(security cms -D -i "$p" | plutil -extract Entitlements.application-identifier raw -)"; done
export APP_PROFILE=<the io.beebeeb.desktop one> EXT_PROFILE=<the io.beebeeb.desktop.FileProvider one>
```

- [ ] **Step 2: Write the helper**

```bash
SCRATCH=$(mktemp -d); H=$SCRATCH/orphan-cleaner; mkdir -p $H
cat > $H/main.swift <<'SWIFT'
import FileProvider
import Foundation

func describe(_ error: Error?) -> String {
    guard let error = error as NSError? else { return "ok" }
    let underlying = (error.userInfo[NSUnderlyingErrorKey] as? NSError).map { " underlying=\($0.domain):\($0.code)" } ?? ""
    return "\(error.domain) \(error.code)\(underlying)"
}

let done = DispatchSemaphore(value: 0)
NSFileProviderManager.getDomainsWithCompletionHandler { domains, error in
    print("before: \(domains.map { "\($0.identifier.rawValue) \"\($0.displayName)\"" }) error: \(describe(error))")
    done.signal()
}
done.wait()
NSFileProviderManager.removeAllDomains { error in
    print("removeAllDomains: \(describe(error))")
    done.signal()
}
done.wait()
NSFileProviderManager.getDomainsWithCompletionHandler { domains, error in
    print("after: \(domains.map { $0.identifier.rawValue }) error: \(describe(error))")
    done.signal()
}
done.wait()
exit(0)
SWIFT
cat > $H/Ext.swift <<'SWIFT'
import FileProvider

/// Empty on purpose: it exists only so the helper app owns the provider id
/// `io.beebeeb.desktop.FileProvider`, which `NSFileProviderManager` derives from the embedded extension.
final class Extension: NSObject, NSFileProviderReplicatedExtension {
    required init(domain: NSFileProviderDomain) { super.init() }
    func invalidate() {}
    func item(for identifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest,
              completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void) -> Progress {
        completionHandler(nil, NSFileProviderError(.noSuchItem)); return Progress()
    }
    func fetchContents(for itemIdentifier: NSFileProviderItemIdentifier, version requestedVersion: NSFileProviderItemVersion?,
                       request: NSFileProviderRequest,
                       completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void) -> Progress {
        completionHandler(nil, nil, NSFileProviderError(.noSuchItem)); return Progress()
    }
    func createItem(basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
                    options: NSFileProviderCreateItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        completionHandler(nil, [], false, NSFileProviderError(.notAuthenticated)); return Progress()
    }
    func modifyItem(_ item: NSFileProviderItem, baseVersion version: NSFileProviderItemVersion, changedFields: NSFileProviderItemFields,
                    contents newContents: URL?, options: NSFileProviderModifyItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        completionHandler(nil, [], false, NSFileProviderError(.notAuthenticated)); return Progress()
    }
    func deleteItem(identifier: NSFileProviderItemIdentifier, baseVersion version: NSFileProviderItemVersion,
                    options: NSFileProviderDeleteItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (Error?) -> Void) -> Progress {
        completionHandler(NSFileProviderError(.notAuthenticated)); return Progress()
    }
    func enumerator(for containerItemIdentifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest) throws -> NSFileProviderEnumerator {
        throw NSFileProviderError(.notAuthenticated)
    }
}
SWIFT
cat > $H/sandbox.entitlements <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>com.apple.security.app-sandbox</key><true/>
  <key>com.apple.security.application-groups</key><array><string>R8352WDJJR.io.beebeeb.desktop.fileprovider</string></array>
</dict></plist>
PLIST
cat > $H/app-Info.plist <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>io.beebeeb.desktop</string>
  <key>CFBundleExecutable</key><string>OrphanCleaner</string>
  <key>CFBundleName</key><string>OrphanCleaner</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
</dict></plist>
PLIST
cat > $H/ext-Info.plist <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>io.beebeeb.desktop.FileProvider</string>
  <key>CFBundleExecutable</key><string>OrphanExt</string>
  <key>CFBundleName</key><string>OrphanExt</string>
  <key>CFBundlePackageType</key><string>XPC!</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSExtension</key><dict>
    <key>NSExtensionPointIdentifier</key><string>com.apple.fileprovider-nonui</string>
    <key>NSExtensionPrincipalClass</key><string>OrphanExt.Extension</string>
    <key>NSExtensionFileProviderDocumentGroup</key><string>R8352WDJJR.io.beebeeb.desktop.fileprovider</string>
    <key>NSExtensionFileProviderSupportsEnumeration</key><true/>
  </dict>
</dict></plist>
PLIST
```

If `swiftc` rejects a signature in `Ext.swift`, copy that method's exact signature from `BeebeebFileProvider/` (the shipping extension) and keep the body as the error it already returns.

- [ ] **Step 3: Build and sign**

```bash
APP=$H/OrphanCleaner.app; EXT=$APP/Contents/PlugIns/OrphanExt.appex
mkdir -p $APP/Contents/MacOS $EXT/Contents/MacOS
cp $H/app-Info.plist $APP/Contents/Info.plist; cp $H/ext-Info.plist $EXT/Contents/Info.plist
xcrun swiftc -target arm64-apple-macos14.0 -O -framework FileProvider $H/main.swift -o $APP/Contents/MacOS/OrphanCleaner
xcrun swiftc -target arm64-apple-macos14.0 -O -parse-as-library -module-name OrphanExt -framework FileProvider \
  -Xlinker -e -Xlinker _NSExtensionMain $H/Ext.swift -o $EXT/Contents/MacOS/OrphanExt
cp "$EXT_PROFILE" $EXT/Contents/embedded.provisionprofile
cp "$APP_PROFILE" $APP/Contents/embedded.provisionprofile
IDENTITY=$(security find-identity -v -p codesigning | awk -F'"' '/Apple Development/ {print $2; exit}'); echo "$IDENTITY"
codesign --force --sign "$IDENTITY" --entitlements $H/sandbox.entitlements --timestamp=none $EXT
codesign --force --sign "$IDENTITY" --entitlements $H/sandbox.entitlements --timestamp=none $APP
codesign --verify --deep --strict --verbose=2 $APP > $EVID/D1-codesign.txt 2>&1; echo "rc=$?"
```

Expected `rc=0`. If signing or the build fails three times in a row, follow Honesty trigger 4: write down what was tried and stop.

- [ ] **Step 4: Run it once**

```bash
ls -la@ ~/Library/CloudStorage/ > $EVID/D1-cloudstorage-before.txt 2>&1
xattr -p com.apple.file-provider-domain-id ~/Library/CloudStorage/Beebeeb-Beebeeb > $EVID/D1-xattr-before.txt 2>&1
ditto $APP /Applications/OrphanCleaner.app
pluginkit -a /Applications/OrphanCleaner.app/Contents/PlugIns/OrphanExt.appex
pluginkit -m -v -i io.beebeeb.desktop.FileProvider > $EVID/D1-pluginkit.txt 2>&1; cat $EVID/D1-pluginkit.txt
date '+%Y-%m-%d %H:%M:%S' > $EVID/D1.helper-start
open -W --stdout $EVID/D1-helper.out --stderr $EVID/D1-helper.err /Applications/OrphanCleaner.app
cat $EVID/D1-helper.out
```

Expected `D1-helper.out`: `before:` lists `io.beebeeb.desktop.domain "Beebeeb"` (the orphan's id, as in its xattr `io.beebeeb.desktop.FileProvider/io.beebeeb.desktop.domain`), then `removeAllDomains: ok`, then `after: []`.

- [ ] **Step 5: The four proofs (spec §11)**

```bash
# 1. The xattr and the Beebeeb-Beebeeb folder are gone (checked BEFORE our add recreates the folder).
xattr -p com.apple.file-provider-domain-id ~/Library/CloudStorage/Beebeeb-Beebeeb > $EVID/D1-xattr-after.txt 2>&1; cat $EVID/D1-xattr-after.txt   # "No such file"
ls ~/Library/CloudStorage | grep -c '^Beebeeb-Beebeeb$'                                                                                     # 0
# 2. fileproviderd no longer writes domain properties for the old provider (window: 10 s after the helper, 5 min).
sleep 300; log show --start "$(date -v-290S '+%Y-%m-%d %H:%M:%S')" --predicate 'process == "fileproviderd" AND eventMessage CONTAINS "io.beebeeb.desktop.FileProvider"' > $EVID/D1-fileproviderd-after.log 2>&1
grep -c "io.beebeeb.desktop.FileProvider" $EVID/D1-fileproviderd-after.log                                                                  # 0
# 3. Our add succeeds: in the QA build, press "Try again" on the folder_taken notice (or relaunch), then:
grep "to=ready" "$LOG" | tail -1 > $EVID/D1-ready.txt; cat $EVID/D1-ready.txt
# 4. BeebeebFileProviderCtl status prints installed.
"$CTL" status > $EVID/D1-ctl-status.txt 2>&1; cat $EVID/D1-ctl-status.txt                                                                   # installed
xattr -p com.apple.file-provider-domain-id ~/Library/CloudStorage/Beebeeb-Beebeeb   # now io.beebeeb.app.FileProvider/io.beebeeb.app.domain
```

- [ ] **Step 6: Delete the helper and the profiles**

```bash
pluginkit -r /Applications/OrphanCleaner.app/Contents/PlugIns/OrphanExt.appex
rm -rf /Applications/OrphanCleaner.app "$SCRATCH"
rm "$APP_PROFILE" "$EXT_PROFILE"
pluginkit -m -i io.beebeeb.desktop.FileProvider | wc -l   # 0
```

Guus deletes the two profiles in the developer portal. If `ls -la@ ~/Library/CloudStorage` on Bram's Mac shows the same xattr, the lead offers him this task (spec §11).

---

## Task 21: Gates and device checks D0–D9 (lead only)

**Lead only.** Lanes never drive the Mac's File Provider, sign Guus out, or touch production. D0–D7 run against the local API with a test account (R3). D8 is the only production action (R7).

- [ ] **Step 1: Decisions queued before the run**

~~Create `.claude/tasks/decisions/<next id>-finder-reauth-keeps-finder.md`. Its question is Spec issue 1 (re-auth goes through `clear_session`, which removes the domain and purges the queue, so R2/D5 cannot pass). It quotes `src/desktopApi.ts:1030-1034` and `src-tauri/src/lib.rs:1802-1844`, and proposes a `reauth` path for spec C. Record Spec issues 6, 11 and 16 in the same file as smaller questions.~~

Round 3 (replaces the round-2 paragraph; issue 1 is answered by R8, issues 6 and 16 are settled in the amended spec, and issue 18 by R9). Create `.claude/tasks/decisions/<next id>-finder-reconciler-open-questions.md` with:
- Spec issue 11 (`/Volumes/*/Applications`).
- ~~Spec issue 17's **Windows** gap: R10 refuses instead of resetting, Windows sign-out refuses while unsent changes exist, and Windows sign-in refuses while a session is present, so a PC in that state has no in-app way forward. Options for Guus: a Windows switch warning plus discard, as on macOS; or keep refusing and document the support path.~~
  Ruled R11: Windows stays fail-closed here, and the escape hatch is private task 1837. Nothing to queue.
- Spec issue 19 (the email fallback and the upgrade adoption), with the security review's answer to question 1 quoted.
- Spec issue 21 (the switch-warning copy, linking the design commit).
- Spec issue 23 (a failed sign-out stops).

Background on R10 stays in the private task. Its Notes get the security review's R10 findings and the D9 results.

- [ ] **Step 2: Final automated gates (each branch in a fresh tree at its head, carrying only its own commits)**

Rust (Lane R head, then again after Lane T's rebase onto merged main). Create the fresh tree with the same command each time, and remove it when done:

```bash
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop fetch origin
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-$TASK-gate origin/feat/$TASK-finder-reconciler
cd ~/code/bb-worktrees/desktop-$TASK-gate/src-tauri
$LOCK cargo-build -- cargo test --locked --no-run > $EVID/final-no-run.log 2>&1; echo "rc=$?"
$LOCK cargo-build -- cargo test --locked > $EVID/final-cargo-test.log 2>&1; echo $? > $EVID/final-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py $EVID/final-cargo-test.log --cargo-exit-code $(cat $EVID/final-cargo-test.exit)
grep -E "test result:" $EVID/final-cargo-test.log | head -3
grep -E "reauth|startup_401|clearing_the_session_token|authorized_probe|caches_the_probed_profile|no_vault_key|holds_vault_key|queued_or_staged_counts" $EVID/final-cargo-test.log | grep -c ' ok$'   # R8 tests that ran: 22 are defined (6 in reauth::tests, 8 in reauth_tests, 2 Task 11 probes, 6 in Task 12); must equal the count in Task 11/12 Notes
grep -E "account_binding" $EVID/final-cargo-test.log | grep -c ' ok$'   # R10 tests that ran: 15 (7 + 8) plus 3 in state_db; must equal Task 10's Notes
grep "test result:" $EVID/final-cargo-test.log           # every binary: ok. N passed; 0 failed
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/final-clippy.log 2>&1
grep -c '^warning' $EVID/final-clippy.log                 # ≤ $EVID/baseline-clippy-warnings.txt
grep -A3 '^warning' $EVID/final-clippy.log | grep -cE "finder_setup/|lifecycle_log.rs|macos_file_provider.rs"   # 0
git -C .. diff origin/main --stat -- src-tauri/Cargo.lock bun.lock   # empty: no new dependency
```

The integration binaries `keychain`, `windows_session_wiring` and `windows_signout_cleanup` run inside `cargo test`. Their `test result:` lines go into Notes, and on Linux CI they are the Windows/Linux proof together with the gated legacy finder tests listed in Task 8 step 3.8.

Frontend (Lane T head). Detach the gate tree onto `origin/feat/$TASK-finder-reconciler-ui` with `git -C ~/code/bb-worktrees/desktop-$TASK-gate checkout --detach origin/feat/$TASK-finder-reconciler-ui`. This tree is the lead's own, allocated above, so a checkout there is allowed.

```bash
cd ~/code/bb-worktrees/desktop-$TASK-gate && bun install --frozen-lockfile && bun test > $EVID/final-bun-test.log 2>&1; echo "rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/final-bun-test.log
bunx tsc --noEmit -p . > $EVID/final-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/final-eslint.log 2>&1; echo "eslint rc=$?"
```

- [ ] **Step 3: Build the QA app (local integration tree, never pushed)**

```bash
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add ~/code/bb-worktrees/desktop-$TASK-qa feat/$TASK-finder-reconciler-ui
QA=~/code/bb-worktrees/desktop-$TASK-qa
git -C $QA merge --no-edit feat/$TASK-finder-reconciler     # local only
security find-identity -v -p codesigning | grep "Apple Development"
for p in ~/Library/MobileDevice/Provisioning\ Profiles/* ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/*; do
  echo "$p $(security cms -D -i "$p" 2>/dev/null | plutil -extract Entitlements.application-identifier raw - 2>/dev/null)"; done | grep "io.beebeeb.app"
export APPLE_SIGNING_IDENTITY="<the Apple Development identity printed above>"
export MACOS_APP_PROVISION_PROFILE="<the io.beebeeb.app dev profile path printed above>"
export MACOS_FILE_PROVIDER_PROVISION_PROFILE="<the io.beebeeb.app.FileProvider dev profile path printed above>"
export BEEBEEB_RELEASE_VERSION=0.8.12-dev.$TASK
cd $QA && bun install --frozen-lockfile
$LOCK cargo-build -- bunx tauri build --debug --target aarch64-apple-darwin --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked > $EVID/qa-build.log 2>&1; echo "rc=$?"
APPQA=$QA/src-tauri/target/aarch64-apple-darwin/debug/bundle/macos/Beebeeb.app
ls $APPQA/Contents/PlugIns/ > $EVID/qa-plugins.txt; codesign -d --entitlements - $APPQA > $EVID/qa-entitlements.txt 2>&1
CTL=$QA/src-tauri/target/fileprovider/BeebeebFileProviderCtl
```

The three angle-bracket values are read from the two listing commands directly above them. They are lookups, not unknowns. Expected: `rc=0`; `qa-plugins.txt` lists the FileProvider `.appex`; the entitlements contain `com.apple.security.app-sandbox`. A `--debug` build is used so Safari's Web Inspector can call `finder_setup_state` for D7.

- [ ] **Step 4: Environment and evidence helpers**

**Precondition since R8: Guus's Mac has no changes waiting to upload.** The QA build shares the installed app's container and state (same bundle id), and its Keychain items when the signing team matches. After the startup 401 his keys stay (Task 12), so the first test-account sign-in in D0 is an **account switch** away from his account. Its warning shows his pending-change count, and "Sign out and switch" removes those changes. Before running the commands below, open his installed app, confirm it reports nothing waiting to upload, and screenshot that (`$EVID/precondition-nothing-pending.png`). If anything is waiting, let it upload first, or stop and ask Guus.

```bash
cd /Users/guuslangelaar/Development/Beebeeb/beebeeb.io && make dev-native     # API :3001, web :5173 (in its own terminal)
curl -sf localhost:3001/health
osascript -e 'quit app "Beebeeb"'; mkdir -p ~/bb-qa/$TASK && mv /Applications/Beebeeb.app ~/bb-qa/$TASK/Beebeeb-installed.app
ditto "$APPQA" /Applications/Beebeeb.app
launchctl setenv BB_API_BASE http://localhost:3001
EVID=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/tasks/_qa-evidence/$TASK
LOG="$HOME/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log"
mark() { date '+%Y-%m-%d %H:%M:%S' > "$EVID/$1.start"; (wc -l < "$LOG" 2>/dev/null || echo 0) | tr -d ' ' > "$EVID/$1.loglines"; }
collect() {
  local n=$1
  tail -n +"$(( $(cat "$EVID/$n.loglines") + 1 ))" "$LOG" > "$EVID/$n-lifecycle.log"
  "$CTL" status > "$EVID/$n-ctl-status.txt" 2>&1
  log show --start "$(cat "$EVID/$n.start")" --predicate 'process == "fileproviderd" AND eventMessage CONTAINS[c] "beebeeb"' > "$EVID/$n-fileproviderd.log" 2>&1
  grep -ci "adding domain" "$EVID/$n-fileproviderd.log" > "$EVID/$n-add-count.txt"
  screencapture -x "$EVID/$n.png"
  echo "$n: ctl=$(tr -d '\n' < "$EVID/$n-ctl-status.txt") adds=$(cat "$EVID/$n-add-count.txt") log_lines=$(wc -l < "$EVID/$n-lifecycle.log")"
}
```

Use the test account and recovery phrase from the `beebeeb:test-accounts` skill (an `@beebeeb.io` address on the local stack). D5b also needs a **second** local test account (the skill's second one, or one made in the local web app; signup is web-only). Guus's own session is signed out by these checks (R3). The first launch of the QA build probes his stored token against the local API, gets a 401, and boots signed out. Restoring it is the last step.

If `$LOG` does not exist after the first launch, the build is not sandboxed. Use `~/Library/Logs/Beebeeb/lifecycle.log` and record that in Notes. In D0, also record whether Console.app → Log Reports lists the file (Spec issue 4), with a screenshot.

The phrase `adding domain` is a guess at fileproviderd's wording. In D0, read `$EVID/D0-fileproviderd.log` and confirm it matches one line per `addDomain`. If it does not, amend the grep pattern in this step in place (strike, replace, sign, date) before D2.

- [ ] **Step 5: Run D0–D7, each between `mark Dn` and `collect Dn`**

| # | Do | Pass means (evidence) |
|---|---|---|
| D0 | Before the cleanup: `open -a /Applications/Beebeeb.app`; sign in with the test account (+ recovery phrase). Since R8 this sign-in may first show the switch warning (it does when the QA build can read Guus's Keychain items, because his account is still on this Mac). If it shows, its sentence must carry no count (D0-switch.png). Then press **Sign out and switch**, and sign in as the test account again. Record whether it showed. | Onboarding shows `An older Beebeeb installation still holds Beebeeb’s place in Finder…` **once**, with **Copy details** (D0.png). `D0-lifecycle.log` has exactly one `transition from=adding to=failed reason=folder_taken attempt=1/1` line: record its `domain=`/`code=`/`underlying=`. `D0-add-count.txt` = 1. Press Copy details, then `pbpaste > $EVID/D0-copy-details.txt; grep -cE '/Users|@' $EVID/D0-copy-details.txt` = 0. **If the domain/code is not `NSCocoaErrorDomain 516` (or underlying `NSPOSIXErrorDomain:17`)**: stop, amend Task 1's row and `COCOA_FILE_WRITE_FILE_EXISTS`, rerun Task 1 steps 7–8, rebuild, repeat D0. In every case, write the captured pair into spec §6.2's `folder_taken` cell (strike the "captured by D0" sentence, add the value + evidence path, sign and date) on the UI branch. |
| D1 | Task 20. | All four proofs of Task 20 step 5. |
| D2 | Sign out (Settings → Account → Sign out). Upload `d2-fixture.txt` (content `D2 fixture`) through the local web app. `mark D2`. Sign in again (password + recovery phrase). Click nothing in Beebeeb after the last sign-in button. | The time from the `signed_in` line to the `to=ready` line in `D2-lifecycle.log` is ≤ 30 s. `cat ~/Library/CloudStorage/Beebeeb-Beebeeb/d2-fixture.txt` prints `D2 fixture` (a cloud-only file opened). |
| D3 | `mark D3; osascript -e 'quit app "Beebeeb"'; sleep 3; open -a /Applications/Beebeeb.app; sleep 20; collect D3` | The last transition is `to=ready`. No onboarding window (D3.png). `D3-add-count.txt` ≤ 1 (§5.5 predicts 0: a registered domain is only confirmed). |
| D4 | Sign out; `"$CTL" status`; `ls ~/Library/CloudStorage`. Then sign in. | After sign-out: `missing`, no `Beebeeb-Beebeeb` (D4-signedout.png of the Finder sidebar). After sign-in: `to=ready` within 30 s, with no click. |
| D4b | Settings → Sync → **Repair…** → **Repair**. | The lifecycle shows `trigger repair`, `ready→missing`, then back to `ready` with no click. `D4b-add-count.txt` ≤ 4. |
| ~~D5~~ | ~~In the local web app (same test account): Settings → sessions → revoke the desktop session. Or use the API: `GET /api/v1/auth/sessions` with the web token, then `DELETE /api/v1/auth/sessions/<desktop id>`. Wait for the desktop's "signed out on this device" banner. `echo "D5 $(date)" >> ~/Library/CloudStorage/Beebeeb-Beebeeb/d5.txt`. Then sign in through the path the app offers.~~ | ~~**Before sign-in:** `"$CTL" status` = `installed` (Finder stays, R2) and the sign-in request is shown. **After sign-in:** per Spec issue 1, today's re-auth path is `clear_session`. Record honestly whether the domain was removed and whether `d5.txt`'s new version reached the server (web version history). If not, D5's line is amended to "blocked by decision <id>". It is not marked passed.~~ |
| D5 | Revoke the desktop session from the local web app (Settings → sessions; or `GET /api/v1/auth/sessions` with the web token, then `DELETE /api/v1/auth/sessions/<desktop id>`). Wait for "You're signed out on this device". `echo "D5 $(date)" >> ~/Library/CloudStorage/Beebeeb-Beebeeb/d5.txt`. Press **Sign in again** and sign in as the **same** test account. Then repeat with a relaunch in between: revoke again, quit, `open -a /Applications/Beebeeb.app`, sign in. | Before sign-in: `"$CTL" status` = `installed` (R2), and the window opens at sign-in. After sign-in: no recovery-phrase step; no `trigger sign_out` and no `ready→missing` line in `D5-lifecycle.log`; `"$CTL" status` = `installed` throughout; the `d5.txt` edit appears as a new version in the web app's version history (screenshot). Relaunch variant: the same, with no recovery phrase (Task 12). If the security review dropped Task 12's discard change, the recovery phrase is asked exactly once and that is the expected result. Record which. |
| D5b | Revoke the desktop session again, as in D5. While it is revoked, with an edit waiting (`echo "D5b" >> ~/Library/CloudStorage/Beebeeb-Beebeeb/d5b.txt`), sign in as a **different** test account. | The switch warning shows a count ≥ 1 (D5b-warning.png). **Cancel**: nothing changed (`"$CTL" status` = `installed`, the banner is still there, no `trigger sign_out`). Do it again, then **Sign out and switch**: `trigger sign_out`, `ready→missing`, `"$CTL" status` = `missing`. The second account then signs in fresh. It is asked for its own recovery phrase, because the first account's vault key is gone (Spec issue 24). Beebeeb comes back for it. The second account's vault on the web lists only its own files (screenshot). |
| D6 | System Settings → General → Login Items & Extensions → File Providers → turn Beebeeb **off**. Open Beebeeb Settings → Sync. Then turn it **on** while that window is visible. | While off: the notice `Beebeeb is turned off in System Settings.` with **Open System Settings** (D6-off.png), and no error surface. After on: `to=ready` with no click in Beebeeb. The add count after the flip is ≤ 1 (§5.5 step 3 predicts 0 app-initiated adds; Spec issue 16). |
| D7 | Sign out; quit. `hdiutil create -volname Beebeeb -srcfolder /Applications/Beebeeb.app -ov -format UDZO $EVID/../qa-$TASK.dmg; hdiutil attach $EVID/../qa-$TASK.dmg; open /Volumes/Beebeeb/Beebeeb.app`. In Safari → Develop → this Mac → Beebeeb, run `await window.__TAURI_INTERNALS__.invoke('finder_setup_state')` in the console. | The lifecycle `launch … location=disk_image`. The invoke returns `reason: "not_in_applications"`, `launch_location: "disk_image"`, before any sign-in (saved to `D7-finder-setup-state.json`). Afterwards: `hdiutil detach /Volumes/Beebeeb; rm $EVID/../qa-$TASK.dmg`. |

**Optional D-S (capture a `signing` code; Spec issue 6).** Quit Beebeeb. Re-sign a copy of the QA app without its provisioning profile: `ditto "$APPQA" ~/bb-qa/$TASK/Beebeeb-adhoc.app; codesign --force --deep --sign - --entitlements src-tauri/entitlements.plist ~/bb-qa/$TASK/Beebeeb-adhoc.app`. Launch it, sign in, and read the failure's `domain=`/`code=` in the lifecycle log. If macOS refuses to launch it, write that down and skip the rung. If a new pair appears, add it to `SIGNING_CODES` with its row in `classify_covers_every_row_of_the_table` (Task 1, mutation-checked) before Lane R merges. Delete the ad-hoc copy afterwards.

**D9: account binding (R10), before and after.** Follow the device check written in the private workspace task: its steps, its two local test accounts and its expected results stay in that private file. Run it twice:
1. **Before**, on a debug build of `origin/main`. Build it with step 3's recipe in its own detached tree (`git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-$TASK-before origin/main`), and install it in place of the QA build for this run only.
2. **After**, on the QA build, reinstalled with `ditto "$APPQA" /Applications/Beebeeb.app`.

Both runs use the local API (`BB_API_BASE` as set in step 4), and the step-4 precondition holds (nothing of Guus's waiting to upload). Evidence goes to the private task's folder under `.claude/tasks/_qa-evidence/` and to `$EVID` (lifecycle lines and screenshots only). Record the outcome in the private task's Notes. Here, write one line: `D9: pass — see the private task` (or the failure). Remove the before-tree afterwards with `git -C … worktree remove ~/code/bb-worktrees/desktop-$TASK-before`.

After D7 and D9, run `launchctl unsetenv BB_API_BASE`.

- [ ] **Step 6: Record in the task file**

Add a `## Verification evidence` section with one line per check (`D0: pass — …`), every count, and the evidence file names. A check that could not run gets its line amended in place, per `.claude/tasks/README.md`.

- [ ] **Step 7: Merge order**

1. Lane R's PR, after its gate, its review, and the `crypto-security-reviewer` run on Tasks 10–12 (Task 11 step 9) with every finding recorded verbatim in Notes (the R10 findings also in the private task) and each one either fixed or answered. Task 12 merges only with the review's OK on keeping keys after a revocation (Spec issue 18). Otherwise its discard change is dropped before the PR, and D5's relaunch variant expects the recovery phrase.
2. Rebase Lane T onto `origin/main` and rerun step 2's frontend gate and the Rust gate in a fresh tree at the rebased head.
3. Lane T's PR. Graphify is regenerated on `main` by the lead after each merge.

- [ ] **Step 8: D8 (production, R7), after the alpha carrying both PRs is published**

```bash
mv /Applications/Beebeeb.app ~/bb-qa/$TASK/Beebeeb-qa.app
gh release download desktop-v0.8.11 -R beebeeb-io/desktop -p '*.dmg' -D ~/bb-qa/$TASK
hdiutil attach ~/bb-qa/$TASK/*0.8.11*.dmg && ditto /Volumes/Beebeeb/Beebeeb.app /Applications/Beebeeb.app && hdiutil detach /Volumes/Beebeeb
open -a /Applications/Beebeeb.app
```

Sign in with the **production** test account from the test-accounts skill. Settings → General → update channel **Alpha** → check for updates → install → the app relaunches. Then `mark D8` just before installing, and `collect D8` 60 s after the relaunch. **Pass:** `update_downloaded`/`update_installed` lines (`from=0.8.11 to=<alpha>`), then `launch …` and `to=ready`, nothing asked (D8.png), and `D8-add-count.txt` ≤ 4. Then sign the test account out (that removes the domain, correctly).

- [ ] **Step 9: Release notes, skills, and restoring Guus's Mac**

- `RELEASE_NOTES.md` for the alpha that carries A, written per `docs/RELEASING.md` before the release workflow runs. It contains What's New (Beebeeb adds itself to Finder; one sentence + one action on failure; the lifecycle log; signing in again after a revoked session keeps everything, and switching accounts warns first), Bug Fixes / Hardening (the folder collision, structured errors, no retry loops, lock/sign-out ordering, local data bound to the account that created it), and Verification with the counted `test result:` and `N pass` lines from step 2.
- `.claude/skills/beebeeb-dev.md` (workspace): one line with the lifecycle log path (container path) under desktop debugging.
- Restore: `rm -rf ~/bb-qa/$TASK/Beebeeb-qa.app`, keep the alpha installed (or reinstall `~/bb-qa/$TASK/Beebeeb-installed.app` if Guus prefers), and run `launchctl getenv BB_API_BASE`, which must print nothing. **Last step:** Guus signs in with his own account (his password). Since R8 that is an account switch away from the test account: the warning shows, then "Sign out and switch", then his password and recovery phrase. The lead confirms `"$CTL" status` = `installed` for his session.

---

## Execution notes for the lead

**Task count: 21.** 12 Rust (Lane R), 7 design/frontend/docs (Lane T), 2 lead-only (20, 21).
- Round 2 (R8) added Tasks 11 and 12 (Lane R) and 18 (Lane T).
- Round 3 (R10) added Task 10 (Lane R) and renumbered everything after it.

| Lane | Tasks | Order | Branch / tree |
|---|---|---|---|
| R | 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 | strictly sequential: each consumes the previous one's types. Then the security review (Task 11 step 9, covering 10–12), then the PR | `feat/$TASK-finder-reconciler`, `~/code/bb-worktrees/desktop-$TASK-r` |
| T | 13 → 14 → 15 → 16 → 17 → 18 → 19 | sequential; Task 13 (design, including the R8 switch warning) is the first commit | `feat/$TASK-finder-reconciler-ui`, `~/code/bb-worktrees/desktop-$TASK-t` |
| lead | 20, 21 | 21 steps 1–3 once both lanes are code-complete; D0 → 20 (=D1) → D2…D7 incl. D5/D5b → D9 → merge → D8 | `desktop-$TASK-qa` (local merge, never pushed) |

- **Two lanes, in parallel** (the cap is two). Lane T needs only the JSON contracts: Task 14 spells out `FinderSetupView`, and Task 18 spells out Task 11's `LoginOutcome`. Lane T never waits for Lane R.
  - Suggested models: Sonnet lanes.
  - Tasks 3, 7, 8, 10 and 11 are the densest. Tasks 10 and 11 are also the security-sensitive ones. The review is a safety net, not a substitute for care.
  - If a lane stalls past ~40 minutes on one task, replace it with a continuation brief that names the last green step.
- **R10 wording.** The Lane R brief for Task 10 tells the lane to read the private task and to write only neutral hardening language in commits, comments and the PR. The lead checks the PR body and every commit message before pushing.
- **Batch boundaries for review:**
  - R1–R3 (pure)
  - R4 (the FFI struct)
  - R5–R7
  - R8–R9 (`lib.rs` Finder wiring)
  - **R10 (the account binding; touches every engine start)**
  - **R11–R12 (auth). The security review on R10–R12 is mandatory before the PR.**
  - T13 (design: Guus may want the artboard and the switch-warning screenshots before T14–T18)
  - T14–T17
  - T18 (the auth UI)
  - T19
- **Where it can go wrong:**
  - Task 8 step 3.7–3.9 (cfg-gating legacy code).
  - The source tests of Tasks 9, 10 and 11. They slice `lib.rs` by text, so a refactor needs the anchors updated.
  - Task 10's Windows branches (`OTHER_ACCOUNT_ON_WINDOWS`) and Task 11's Windows-only neighbours (`attempt.run(work)`, `SESSION_TRANSITION`). The Mac cannot run them; CI's Windows job is the check.
  - D0 (the classifier row is provisional until it runs).
  - The D5 relaunch variant, which depends on Task 12 surviving the security review.
  - D9, which needs a second build (`origin/main`).
- **Decisions for Guus** (Task 21 step 1): Spec issues 11, 19, 21 and 23. The Windows gap (17) is ruled by R11, and its escape hatch is private task 1837.

### Spec coverage map

| Spec | Task(s) |
|---|---|
| §2 correction of 1696/1698 | 4 (both code comments), 19 (CLAUDE.md) |
| §2 finding 7 (re-auth data loss) | 11, 12, 18 |
| §2 finding 8 and §5.6 (R10, local data belongs to one account) | 10 (binding, every engine start, upgrade adoption, the stopping purge), 11 (R8 records the owner), 12 (the probed profile), D9 |
| §3 R8 | 11 (account check, token swap, every sign-in method), 12 (startup 401 keeps keys, probed profile), 18 (banner, sign-in routing, switch warning), 13 (warning drawn first) |
| §3 R9 | 12 (merges only with the review's OK), 11 step 9 question 5 |
| §5.1 wanted table, R2, R8 rows | 3 (`wanted`, matrix test), 6 (`finder_signed_out_by_choice`), 8 (`session_facts`), 11, 12 |
| §5.2 states + `finder_reason` | 1 (enum), 7 (view), 8 (`PopoverSnapshot.finder_reason`, popover) |
| §5.3 triggers (keys-arrive list, sign-out first, lock) | 8 (launch), 9 (keys, sign-out, lock, Repair, app active), 11 (re-sign-in loads keys), 3 (try again, flip) |
| §5.4 one check, merged triggers | 3 (`merged_triggers…`), 7 (one op at a time, events between ops) |
| §5.5 one check; engine before add; event + log line per transition | 3, 7, 8 (`MacosPorts`, `ensure_sync_root_and_engine`) |
| §6.1 structured bridge errors; no substring matching on macOS | 4, 8 (legacy classifier cfg'd off macOS) |
| §6.2 table (folder_taken pair, empty signing set, -2005/-2013), copy, Copy details, launch location, Show in Finder | 1, 2, 7 (`copy_details_text`), 8 (`finder_setup_show_app`), 14 |
| §7 retry policy, injected clock, UserDisabled poll | 1, 3, 7 |
| §8 lifecycle log (sandboxed path) | 5, 8 (init + launch line), 9 (sign-in/out, updates, support bundle), 19 (docs) |
| §9 units, commands, config | 1–8, 6, 10 (`account_binding`), 11 (`reauth`, `open_reauth_window`, `LoginOutcome`) |
| §10 frontend (incl. the SettingsView panel and R8) | 14–18 |
| §11 orphan helper | 20 |
| §12 design artefacts first (incl. Repair copy, switch warning) | 13 |
| §13.1 automated tests + gates (no new clippy warnings vs baseline; R8 and R10 tests; security review) | every task; 11 step 9; 21 step 2 |
| §13.2 device checks D0–D9 incl. D5/D5b | 21 |
| §14 docs | 19, 21 step 9 |
| §15 open items | D0 (folder_taken), Task 20 step 1 (App IDs), Spec issues 6, 11, 17 (R11), 18 (R9), 19, 23, 24 |
