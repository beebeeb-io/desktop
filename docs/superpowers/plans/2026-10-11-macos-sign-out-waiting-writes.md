# macOS Sign-out Keeps Waiting Finder Saves: Implementation Plan (task 1887)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On a Mac, a sign-out by choice never discards a Finder save that is still waiting to upload. It sends first (at most 60 s), then asks with the real count, then copies every waiting save into a kept folder on this Mac before anything is purged, and surfaces that folder as 1882 surfaces macOS's.

**Architecture:** One `StateDb` read classifies the queue into waiting files (copied, covered or missing per write) and other waiting changes. The webview's ask is a snapshot Rust holds. `clear_session_impl` gates on it before anything changes. After the confirmed engine stop, a keep step closes Finder accepts, copies the copied set with `clonefile` into `<container>/Data/Documents/Changes not sent/<date at time>/…`, flushes, and records the folder durably. The purge refuses, before it deletes any file, if any waiting write is not in the keep step's `KeptSet`. Around that core sit a Rust-owned send phase (a third arm in the runner's `select!`), a window-close hook with a pending late result, the flow commands, and the frontend flow in Settings › Account, the compact page and the switch step.

**Tech Stack:** Rust 2024 (Tauri v2 daemon; rusqlite bundled SQLite; tokio; `libc` 0.2.189 for `clonefile`; `chrono` for the event folder's local time; no new crate), React 19 + TypeScript (bun test with the repo's `componentHarness`), Markdown and the hifi HTML for the design-first commit. No Swift change is planned.

**Spec:** `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`, revision 5 (commit `10c4bf41`, on `184ac962`). Executors read the spec and this plan together. Rule, test and step ids (W5, U37, F11, D7, §5.2 step 8) are the spec's. `@184:` cites desktop at `184ac962`. Where the plan and the spec differ, the reconciliation table below says why, and nothing is decided silently.

---

## Open questions for the lead (answer before Tasks 7 and 11 start)

Both are open questions 3 and 4 in the spec's §15, with the full reasoning there. The rest of the plan does not depend on the answers. Each answer changes one task, as stated.

**Q1 (spec question 3): close Finder accepts before the keep step's read?**
A Finder accept that carries a `request_id` runs its copy and its transaction in `spawn_blocking` (`@184:ipc_socket.rs:2448-2479`, `@184:ipc_write_dedup.rs:336-342`). The engine stop aborts the connection tasks without awaiting them (`@184:ipc_socket.rs:1673-1675`), and 1873 says so itself (`@184:runner.rs:1025-1030`). So an accept can commit after the keep step's read. If it commits after the purge guard's read, the purge deletes that waiting write unkept, and only macOS's own copy remains. That breaks W1.
- **Option A (recommended): close them.** The keep step's first act writes `sync_state.finder_accepts_closed` in one `BEGIN IMMEDIATE` transaction. `accept_finder_write` reads it inside its own transaction and answers `AcceptsClosed`, which becomes `WriteRetryLater` (`accepts_closed`), exactly as `CopyGone` does today. `clear_account_data` and `engine_start_repair` clear it. One key, one read, one delete, U45. No Swift change.
- **Option B: amend the description only.** The late accept stays. The guard catches some cases (E2, nothing deleted), and in the rest W1 relies on macOS keeping an unacknowledged change.
- **Recommendation: A.** The case B leaves is exactly what W1 forbids, with an unverified macOS behaviour as its only cover, and A reuses 1873's own refusal path.
- **Effect on the plan:** Task 7 implements A. Under B, skip Task 7, and in Task 20 add the sentence from the spec's §15 option B to W1.

**Q2 (spec question 4): end the send phase on a refusal the account cures?**
For a Finder write, `quota_exceeded`, `object_budget_exceeded`, `billing_read_only`, `account_lapsed`, `plan_required` and `trial_cancelled_read_only` are transient: the write stays live (`is_curable_refusal`, `@184:engine_bridge.rs:733-754`, applied at `:1187`). As written, the phase retries it, ends `time_limit` at 60 s, and A5 says "Choose Cancel to let it finish, then sign out again.", which is not true until the person frees space or chooses a plan.
- **Option A (recommended): a reason of its own.** The first attempt during the phase that meets one of those refusals ends it with `account`. A5 gains a line, proposed: "The server isn’t accepting uploads for this account until it has free space or an active plan." U46 and an F2 case pin it.
- **Option B: no new reason.** The phase runs 60 s; only A5 `time_limit`'s second sentence changes, for example to "Choose Cancel to keep it waiting, then sign out again."
- **Recommendation: A.** About one second instead of sixty, and a line that says what would let the change through.
- **Effect on the plan:** Task 11 implements A and Task 16 adds the line. Under B, Task 11 drops the `Account` end and U46 (steps marked "Q2-A"), and Task 16 changes the `time_limit` string instead of adding one.

---

## Reconciliation: spec text → code at `184ac962` → amendment

The spec was written before 1873's code was final. 1873 merged after fourteen final-fix rounds; 1885 and 1889 merged too. Every row below is amended in the spec in place (old text struck, new text signed "— lead, 2026-10-11 (plan)"), commit `10c4bf41`, unless the last column says no amendment.

| # | Spec text (where) | What the code does at `184ac962` | Amendment |
|---|---|---|---|
| 1 | Header: "It gates the release of task 1873: 1873 may merge, but it is not released until this ships." | 1873 merged (`87c11c25`). `RELEASE_BLOCKERS.md` lists this gap. `release.yml:110-126` and `scripts/release-macos-local.sh:502-512` refuse while it lists an item | Header line struck and replaced; §14 gains the item's removal after the device rung |
| 2 | "Read at" | `lib.rs` grew to 32.5k lines; every `main:` line number names `af9b351` | A "Revision 5 (plan)" entry: `@184:` citations live in this plan; newly read files listed |
| 3 | §1 item 2: "a queued save is not that" (what 1882 keeps) | Device, 2026-10-11, QA build of 1873's `42b7bef8`: macOS kept a PARKED save's bytes (`preserved=true`) | Device evidence added; the unparked case stays D2's question |
| 4 | §2 "Live" | A park still sets `attempts = max_attempts` (`@184:state_db.rs:1328`, `:5476`). A Finder write is paused only for `auth` or held `key_replaced`; a 403 parks it (`permission_denied`); a transient failure never pauses or parks it (`failed_attempt_rule`, `@184:engine_bridge.rs:865-891`) | One signed sentence added to the row |
| 5 | §5.2 step 6: "After a confirmed stop no IPC listener and no runner exist, so the queue cannot change until the purge." | A `spawn_blocking` accept outlives the stop (Q1 above) | Struck and replaced; three cases named; question 3 |
| 6 | §5.2 "What must hold (re-review O1)" | Same | A signed paragraph: the one writer outside `clear_session_impl` |
| 7 | §5.3 row "8. Purge guard": "Only a test seam can cause it" | Same | Struck inside the cell; replaced |
| 8 | §6.2 "Why the margin is 2": "Every failure that is not a pause adds one attempt" | A Finder write's transient failure adds one only below `backoff_cap_attempts()` = 6 and never past `max_attempts - 1` (`next_attempt_count`, `@184:engine_bridge.rs:836-850`); `retry_delay_seconds` unchanged (`:6708-6711`) | Struck and replaced; the margin rule itself holds |
| 9 | §6.4 (no case for an account refusal) | Curable refusals are transient for a Finder write (`@184:engine_bridge.rs:733-754`, `:1187`) | A signed paragraph; question 4 |
| 10 | §7.1 "U12 fails until both lists hold the new root" | Both hold it: `FinderStagingBases::candidates()` (`@184:engine_bridge.rs:6235-6301`, data root under `cfg(macos)`), `finder_staging_candidates()` (`:6398-6400`), `disposable_cache_roots()` (`@184:lib.rs:4000-4029`) | "Done in 1873" added |
| 11 | §7.1 cores `disposable_cache_roots_in(temp_dir, cache_dir, data_dir, hydrate_cache_dir)` and `finder_staging_candidates_in(cache_base, data_dir, temp_dir)` | The staging core exists as `FinderStagingBases { data, cache, temp }.candidates()`; `disposable_cache_roots()` has no core and lists staging folders, never `data_dir` | Signatures struck; `disposable_cache_roots_in(temp, cache, hydrate, &FinderStagingBases)` |
| 12 | §7.1 "the engine-start repair sweeps (`staged_payloads`)" | Two engine-start steps: `engine_start_repair` (`@184:runner.rs:1007-1024`) and `staged_payload::sweep_unreferenced` (`:1031-1053`; `@184:staged_payload.rs:197`) | A signed paragraph; U12 names both |
| 13 | §7.2 "`files.path`, for example `/Work/report.docx`" | Placement by `parent_id`: `files.path` is the full relative path under the parent, no leading slash (`relative_path_under_conn`, `@184:state_db.rs:1010`); a re-homed row has its name alone (`rehome_kept_row_conn`, `:1070-1102`); a folder renamed after its children were written leaves their old prefix | Example struck; the two cases accepted and named |
| 14 | §7.3 "`save_to` … never calls `sync_all` (`main:config.rs:662-679`)"; U43 "(`save`, `update_at`)" | Task 1889: no public `save()`. One write path, `try_update_at_then` (`@184:config.rs:806-822`) → private `save_to` (`:825-843`, one production call at `:816`) | Signed paragraph; U43's list struck and replaced |
| 15 | §8 item 4: "The parked write is covered" | 1873 (4d): when W parked `payload_missing`, N was built on the server's version, not on W's bytes | Exception named; no rule change (same outcome as the hand-over) |
| 16 | §8 item 7: the journal row with no op | Engine start deletes it, guarded (`forget_unfinished_unreferenced`, `@184:state_db.rs:4659`), then its copy; the accept requires the row (`@184:state_db.rs:3732-3743`); the macOS switch count drops released copies (`@184:state_db.rs:4760-4779`) | A signed paragraph |
| 17 | §9 "Unverified" (does macOS keep a queued write) | Device: a parked save was kept. 1882 surfaces only a folder that exists and holds an entry (`kept_folder_from_bridge`, `@184:finder_removal.rs:121`) | Two signed paragraphs |
| 18 | U2 "a quota-paused write" | Quota never pauses a Finder write | Row struck; a new U2 with `permission_denied`, `auth`, `key_replaced` and a quota-paused `create_folder` |
| 19 | U10 rationale | The late accept is invisible to a source pin | Sentence added; U45 |
| 20 | U12 "once 1873 has it" | 1873 has it; the data root is macOS only | Sentence added |
| 21 | §13.3 gates | Count guard; baseline lib `1763 passed; 6 ignored`, integration 31 / 8 / 3, guard 1805, Swift 102, release shim 410 (1873 final gate at `883e7669`) | Two bullets added |
| 22 | §13.4 required rungs | Device note 2026-10-10: a `create_folder` held `key_replaced` was purged with no message | D11 added; D2 records the parked and unparked cases; `RELEASE_BLOCKERS.md` only after D1-D11 and D10b |
| 23 | §14 | `RELEASE_BLOCKERS.md` | A bullet: its removal is its own change, after the rung |
| 24 | §15 | Two design gaps | Questions 3 and 4 added |
| — | §2 other change kinds | 1873's three-state parent (`ParentChange`, `@184:api_client.rs:367`): a move to the top level is now `move_file`, before `rename_file` | No amendment: both are other waiting changes, counted the same |
| — | §1, §4 rows 1-8, §5.1 menu dialogs, §9 1882 records | Same shapes at new lines: `clear_session_impl` `@184:lib.rs:2823`, `clear_session` `:3312-3324`, menu `:13994-14040`, purge `:3260-3293`, `:4216-4265`; `MacSettings.tsx:317-324`, `:458`, `:466-475`; `pages/Account.tsx:148-166`, `:367-370`; `Onboarding.tsx:149-152`, `:472-503`; `finder_removal.rs:13`, `:17-18`, `:276-307`, `:362-389` | No amendment: the descriptions hold; the plan cites `@184:` |
| — | §6.2 "the queue step runs only inside a successful tick" | `select!` `@184:runner.rs:1299`, pause check `:1357`, tick `:1395`, queue step `:1421` | No amendment |

---

## Global Constraints

Copied from the spec; each line is verbatim or names its section.

- **macOS only** (W5, §10): "the gate, `keepUnsent`, the flow commands, the callers' handling, the keep step and the purge guard are `#[cfg(target_os = "macos")]`. Windows and Linux keep today's sign-out, switch and count." So are the marker's save and clear, the pending late result and `sign_out_result_ack`.
- **One count, never 0** (W2): "Every number a person sees comes from one function, `waiting_changes`, read in one `StateDb` call … A sentence whose number would be 0 is left out. … A failed read is never a 0: it stops the flow (E3)."
- **The bound** (§6.3): "`SIGN_OUT_SEND_LIMIT = 60 s`, from the start of the phase. One constant, read through an injected clock in the tests."
- **Where** (§7.1): `<the app's container>/Data/Documents/Changes not sent/<YYYY-MM-DD> at <HH.MM.SS>/<the file's path in the vault>`; outside the sandbox `<data_dir>/beebeeb/Changes not sent`. "If the folder exists, " (2)", " (3)" and so on are added." "The folder is made only when at least one file is copied."
- **No copy can overwrite** (§7.2): "`std::fs::copy` must not be used." `libc::clonefile(src, dst, CLONE_NOFOLLOW)` with `const CLONE_NOFOLLOW: u32 = 0x0001;`; on `ENOTSUP` or `EXDEV`, `create_new` then `std::io::copy`; `EEXIST` takes the next " (n)".
- **Path length** (§7.2): a destination whose absolute path is "1024 UTF-8 bytes or more" goes to the top of the event folder.
- **Copy** (§11): "All strings use the typographic apostrophe (’) and the single-character ellipsis (…). No provider name, no emoji, no "don’t worry". Every string is one constant, mirrored in Rust and in `src/macSettingsModel.ts` where both sides need it, with a test pinning the two equal." "`{n}` is never 0."
- **Errors** (§11): "`<code>: <sentence>`, and a caller matches the code, never the sentence." Codes `unsent_changes`, `unsent_copy_failed`, `unsent_count_failed`.
- **Logs** (§6.4, §7.2, W10): counts only, "No path and no name". "Logs never carry its path or a file name."
- **The folder is the person's** (W10): "After the keep step completes, Beebeeb never moves, reads back, deletes or uploads the kept folder."
- **Design first** (§11): "the first commit of the frontend task, before any frontend code, amends" Dialog 3, the hifi drawing and spec A. "The frontend pull request does not merge without that commit."
- **No new dependency** (§13.3): "`src-tauri/Cargo.lock` and `bun.lock` unchanged".
- **Public repository** (workspace rule): neutral wording in commits, comments and docs; no private names or ledger labels; each commit ends with the trailer of the model that wrote it.
- **Evidence** (§13): "Every new test is seen red before it passes, and its mutation is seen red on the intended assertion. Failure and pass output go into the task's notes and QA evidence." Counts, never adjectives.

## Review Focus

The five inputs most likely to bite a person that no U, F or D test in the spec exercises. Each has a test, added to the task that owns the code.

1. **Two sign-outs from one ask** (a double click on "Sign out", or ⌘⇧L while Settings is signing out). A person expects one sign-out and one kept folder. Pinned by `a_second_sign_out_with_the_same_ask_keeps_nothing_twice` (Task 8): the second call answers `unsent_changes:` because the first took the snapshot.
2. **A retry in the same second as a failed keep.** A person expects a fresh folder, never files added to a half-made one. Pinned by `two_keeps_in_the_same_second_make_two_folders` (Task 3).
3. **An empty save (0 bytes) waiting.** A person expects an empty file in the kept folder, not "missing". Pinned by `an_empty_save_is_kept_as_an_empty_file` (Task 5).
4. **A save and a rename of the same file, both waiting.** A person expects one file in the ask plus one other change, and the kept copy under the name Finder shows. Pinned by `a_waiting_rename_names_the_kept_copy_as_finder_shows_it` (Task 5): the name comes from the row's path at the keep step, never from an op's `target_path`.
5. **A bulk drop: 500 new files in one Finder folder, all waiting.** A person expects "500 files", 500 kept files and no lost name. Pinned by `five_hundred_waiting_files_are_counted_and_kept_once_each` (Task 5).

---

## File map

Paths are relative to the desktop repo root.

| File | Status | Responsibility | Tasks |
|---|---|---|---|
| `src-tauri/src/sign_out/mod.rs` | create | the module list | 1 |
| `src-tauri/src/sign_out/waiting.rs` | create | `WaitingChanges`: classify rows into files (copied, covered, missing) and other changes; counts; liveness | 1 |
| `src-tauri/src/sign_out/kept_names.rs` | create | `kept_root`, `kept_relative_path`, component cleaning, the 255-byte cut, `event_folder_name` | 2 |
| `src-tauri/src/sign_out/kept_writer.rs` | create (macOS) | the event folder, directories one at a time, `clonefile` with the collision and length rules, flushes, the backup attribute | 3 |
| `src-tauri/src/sign_out/kept_set.rs` | create | `KeptSet`, `Acknowledge` | 5 |
| `src-tauri/src/sign_out/kept_record.rs` | create | the two config keys: record, take back, clear, dismiss | 4 |
| `src-tauri/src/sign_out/keep.rs` | create (macOS) | the keep step | 5 |
| `src-tauri/src/sign_out/snapshot.rs` | create | `AskSnapshot`, `SnapshotStore`, the gate's comparison | 8 |
| `src-tauri/src/sign_out/copy.rs` | create | every §11 string and its builders (Rust side) | 10 |
| `src-tauri/src/sign_out/send_phase.rs` | create | the send phase's pure state and end decision | 11 |
| `src-tauri/src/sign_out/flow_window.rs` | create | the late-result record and the close hook's pure state | 12 |
| `src-tauri/src/state_db.rs` | modify | `waiting_rows`, `unkept_waiting_changes`, `close_finder_accepts`, the accept's check, `engine_start_repair`'s reopen, `make_due_for_send_phase` | 1, 6, 7, 11 |
| `src-tauri/src/config.rs` | modify | the two keys; `update_at_durable` with a file-system seam | 4 |
| `src-tauri/src/lib.rs` | modify | `mod sign_out;`; `disposable_cache_roots_in`; the purge's `KeptSet` and order; the gate and keep call in `clear_session_impl`; the commands; the alert; the window hook; the menu; the R10 reset; `AppState.sign_out` | 2, 6, 8, 10-14 |
| `src-tauri/src/engine_bridge.rs` | modify | `AcceptsClosed` → `WriteRetryLater`; the queue outcome's link, session and account signals | 7, 11 |
| `src-tauri/src/ipc_socket.rs` | modify | the `accepts_closed` reply and refusal category | 7 |
| `src-tauri/src/runner.rs` | modify | `SendPhaseLink` in `EngineRunner` and `RunnerControls`; the third `select!` arm | 11 |
| `src-tauri/src/browser_login.rs` | modify | the switch event carries the snapshot on macOS | 13 |
| `docs/specs/2026-10-02-macos-settings-dialogs.md`, `design/hifi/macos-settings-dialogs.html`, `docs/specs/2026-10-06-macos-finder-setup-reconciler.md` | modify | design first | 15 |
| `src/signOutCopy.ts` | create | the §11 strings and builders (TypeScript side) | 16 |
| `src/signOutFlow.ts` | create | the Settings flow's pure state machine | 17 |
| `src/MacSettings.tsx` | modify | the Account tab runs the flow; the Sync tab gets our row | 17, 19 |
| `src/pages/Account.tsx`, `src/Onboarding.tsx`, `src/onboardingSignIn.ts`, `src/accountSwitchCopy.ts`, `src/desktopApi.ts` | modify | the compact page, the switch step, the result's `unsent`, `clearSession`'s `keepUnsent` | 18 |
| `tests/signOutCopy.test.ts`, `tests/signOutFlow.test.ts`, `tests/signOutEntryPoints.test.tsx`, `tests/keptUnsentRow.test.tsx` | create | F1-F12 | 16-19 |
| `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md`, `docs/specs/2026-10-09-macos-finder-write-versions.md`, `CLAUDE.md`, `docs/IPC_PROTOCOL.md`, `RELEASE_NOTES.md` | modify | §14 | 20 |
| `RELEASE_BLOCKERS.md` | modify | remove the item, after the rung | 22 |

## Lane, branch and conventions

- **One lane for Tasks 1-14, in order** (each consumes the previous one's types). Tasks 15-19 (frontend) may run as a second lane once Task 13 is committed, because every command and payload they call is fixed by then. At most two lanes. Tasks 21 and 22 are the lead's.
- **Tree:** `~/code/bb-worktrees/desktop-1887`, branch `fix/1887-sign-out-waiting-writes`, which already carries the spec and this plan. A second lane takes its own tree from this branch's head (`make worktree N=1887-ts` from the workspace root).
- **Variables used in every task:**

```bash
WT=~/code/bb-worktrees/desktop-1887
LOCK=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/scripts/coord/with-lock.sh
EVID=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/tasks/_qa-evidence/1887
mkdir -p $EVID
```

- **The test loop** (one filter, from `$WT/src-tauri`):

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked --lib <filter> > $EVID/t<N>-<step>.log 2>&1; echo "rc=$?"; grep -E "test result:|error\[|FAILED|panicked" $EVID/t<N>-<step>.log | head -20
```

  `rc=75` means the lock was busy: run it again. A filter that matches nothing prints `0 passed`: that is a red.
- **The Rust task gate** (end of every Rust task; `<L>` is the lib count the task names, `L0` is Task 1's measured baseline):

```bash
cd $WT/src-tauri && cargo fmt --check > $EVID/t<N>-fmt.log 2>&1; echo "fmt rc=$?"
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t<N>-check.log 2>&1; echo "check rc=$?"
$LOCK cargo-build -- cargo test --locked > $EVID/t<N>-cargo-test.log 2>&1; echo $? > $EVID/t<N>-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py $EVID/t<N>-cargo-test.log --cargo-exit-code "$(cat $EVID/t<N>-cargo-test.exit)"; echo "guard rc=$?"
grep "test result:" $EVID/t<N>-cargo-test.log
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t<N>-clippy.log 2>&1; grep -c "^warning" $EVID/t<N>-clippy.log
```

  Expected: `fmt rc=0`, `check rc=0`, `guard rc=0`; the lib line reads `ok. <L> passed; 0 failed; <I> ignored` with `<I>` the baseline's ignored count; `keychain`, `windows_session_wiring` and `windows_signout_cleanup` keep their baseline counts; clippy's warning count is not above the baseline's.
- **The frontend gate** (from `$WT`):

```bash
cd $WT && bun test > $EVID/t<N>-bun.log 2>&1; echo "bun rc=$?"; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t<N>-bun.log
bunx tsc --noEmit -p . > $EVID/t<N>-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/t<N>-lint.log 2>&1; echo "lint rc=$?"
```

  Expected: `N pass` where N is the task's stated count, `0 fail`, `tsc rc=0`, `lint rc=0`.
- **Mutation rule.** Every new test is seen RED before its code exists (step "run, expect FAIL"), then GREEN, then each named mutation is applied, seen RED on the named assertion, and reverted. Paste the RED line and the GREEN line into the task file's Notes. A mutation that turns a different assertion red does not count.
- **Gate forms** (W5). A behaviour only macOS has is `#[cfg(target_os = "macos")]`, and so are its tests. Pure modules compile everywhere and carry `#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]` at their top, as `ipc_socket.rs` does for its helpers. Lib counts are macOS counts. Linux and Windows CI run the non-macOS half (U32).
- **Process.** Foreground commands only. No `git stash`, `reset`, `checkout --`, `restore`, `clean`, `switch`. No `pkill`. Explicit pathspecs on every commit. No network from a lane (the lead fetches and pushes). Each commit ends with the trailer of the model that wrote it.

---

## Task list

| # | Task | Size | Lane | Spec |
|---|---|---|---|---|
| 1 | The waiting-changes read, and when a phase can send | M | Rust | §2, W2, §6.1; U1-U5, U15, U38 |
| 2 | Kept names and roots | M | Rust | §7.1, §7.2; U11 (names), U12, U33 |
| 3 | The kept-folder writer | L | Rust | §7.2 "How"; U11 (disk), U28, U40 |
| 4 | The two config keys and the durable save | M | Rust | W9, W13, §7.3; U22, U43 |
| 5 | The keep step | L | Rust | W6-W9, §5.2 step 7; U8, U8b, U13, U27, U29a, U36, U38 |
| 6 | The purge guard and its order | M | Rust | §5.2 step 8, W13; U14, U39 |
| 7 | Close Finder accepts before the keep step (Q1-A) | M | Rust | spec question 3; U45 |
| 8 | The ask snapshot, the gate and the sign-out | L | Rust | W5, §5.2 steps 4-10; U6, U7, U9, U10, U29b-c, U30, U32, U37, U41 |
| 9 | An interrupted sign-out, by a real process death | M | Rust | W13; U42 |
| 10 | The alert, the row's commands, "Show in Finder", logs, Rust copy | M | Rust | W9, §9, §11; U23-U26, U31 |
| 11 | The send phase | L | Rust | §6; U16-U18, U34a/f, U35, U46 |
| 12 | A close ends the flow; a late result is shown | L | Rust | §5.1; U34b-e/g/h, U44 |
| 13 | Entry points: the flow request, the menu, the switch's snapshot | M | Rust | §4 rows 2, 5, 6; U19, U21 |
| 14 | The R10 reset keeps before it purges | S | Rust | W12; U20 |
| 15 | Design first: Dialog 3, dialog 4, spec A | M | docs | §11 "The design changes first", §14 |
| 16 | Frontend copy and its mirror | M | TS | §11; F2, F6, F7 |
| 17 | The Settings flow | L | TS | §5.1; F1, F3, F9-F12 |
| 18 | The compact page, the switch step, `forceReauth` | M | TS | §4 rows 2-4, 6; F4, F8, F12 |
| 19 | Settings › Sync: our row | S | TS | §9; F5, K2, K3 |
| 20 | Docs that change with the code, and the branch gate | M | docs | §13.3, §14 |
| 21 | The device rung (lead only) | L | lead | §13.4 D1-D11, D10b |
| 22 | Remove the release blocker (lead only, after Task 21) | S | lead | §14 |

Sizes: S about an hour, M about half a day, L about a day, each with its mutations. 22 tasks.

---

### Task 1: The waiting-changes read, and when a phase can send

**Files:**
- Create: `src-tauri/src/sign_out/mod.rs`, `src-tauri/src/sign_out/waiting.rs`
- Modify: `src-tauri/src/lib.rs:88` (add `mod sign_out;` after `mod runner;`)
- Modify: `src-tauri/src/state_db.rs` (add `WaitingRow` and `StateDb::waiting_rows` after `pending_changes_count`, `@184:state_db.rs:4760-4779`)
- Test: `src-tauri/src/sign_out/waiting.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `StateDb`, `OperationKind`, `WriteOrigin::from_db`, `StateDb::hold_lock_for_test` (tests).
- Produces (later tasks rely on these exact names):
  - `state_db::WaitingRow { op_id, kind, file_id, file_path, payload_path, origin, queue_order, attempts, max_attempts, paused, completed }` and `StateDb::waiting_rows(&self) -> rusqlite::Result<Vec<WaitingRow>>`.
  - `sign_out::waiting::{WaitingChanges, WaitingFile, WaitingWrite, WriteState, OtherChange, WaitingCounts, NotSending, SendConditions, send_readiness, staged_copy_exists}`.
  - `WaitingChanges::read(&StateDb) -> Result<WaitingChanges, String>`, `::classify(Vec<WaitingRow>, impl Fn(&Path) -> bool)`, `.counts() -> WaitingCounts`, `.is_empty()`, `.changes() -> usize`, `.any_live()`, `.file_ids() -> BTreeSet<String>`, `.other_op_ids() -> BTreeSet<String>`, `.missing_op_ids() -> BTreeSet<String>`.

- [ ] **Step 0: Baselines.** Before any change, at the branch head:

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked > $EVID/t0-cargo-test.log 2>&1; echo $? > $EVID/t0-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py --self-test; python3 ../scripts/assert-cargo-test-counts.py $EVID/t0-cargo-test.log --cargo-exit-code "$(cat $EVID/t0-cargo-test.exit)"
grep "test result:" $EVID/t0-cargo-test.log
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t0-clippy.log 2>&1; grep -c "^warning" $EVID/t0-clippy.log
cd $WT && bun test > $EVID/t0-bun.log 2>&1; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t0-bun.log
bash scripts/test-ipc-framing.sh > $EVID/t0-swift.log 2>&1; tail -3 $EVID/t0-swift.log
```

Expected near 1873's final gate (`883e7669`): lib `ok. 1763 passed; 0 failed; 6 ignored`, `keychain` 31, `windows_session_wiring` 8, `windows_signout_cleanup` 3; Swift `ipc-framing: 102 passed, 0 failed`. Write the measured numbers into the task file's Notes under "Baseline (184ac962)": `L0` (lib passed), `I0` (lib ignored), `C0` (clippy warnings), `B0` (bun pass). Every later expected count is written as `L0 + k`.

- [ ] **Step 1: Write the failing tests.** Create `src-tauri/src/sign_out/mod.rs`:

```rust
//! Spec 2026-10-10 (task 1887): a sign-out by choice never loses a Finder save that is still waiting.
//! The pure parts compile on every platform; the keep step and the folder writer are macOS only (W5).

pub(crate) mod waiting;
```

Add `mod sign_out;` to `lib.rs` after `mod runner;` (`@184:lib.rs:88`). Create `src-tauri/src/sign_out/waiting.rs` holding only the test module below. It does not compile yet: that is Step 2's RED.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::StateDb;
    use std::path::Path;

    fn db_with(sql: &str) -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let sql = sql.replace("<dir>", dir.path().to_str().unwrap());
        db.hold_lock_for_test().execute_batch(&sql).unwrap();
        (dir, db)
    }

    fn stage(dir: &Path, names: &[&str]) {
        for name in names {
            std::fs::write(dir.join(name), name.as_bytes()).unwrap();
        }
    }

    const COMPLETED: &str = "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns,
        upload_session_id, server_file_id, object_version_id, chunk_size_bytes, chunk_count, completed_version)
        VALUES ('d1', '<dir>/d1', 1, 1, 'u', 'f3', 'v', 1, 1, 7);";

    /// U1.
    #[test]
    fn waiting_changes_counts_files_and_other_changes_once() {
        let (dir, db) = db_with(&format!(
            "INSERT INTO files (file_id, path) VALUES ('f1', 'Work/report.docx'), ('f2', 'New/notes.txt'), ('f3', 'Done.txt');
             INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
               ('s1', 'upload_version', 'f1', '<dir>/s1', 'w1', 'minted'),
               ('s2', 'upload_version', 'f1', '<dir>/s2', 'w2', 'minted'),
               ('c1', 'upload_version', 'f2', '<dir>/c1', 'w3', 'minted'),
               ('r1', 'rename_file', 'f1', NULL, NULL, NULL),
               ('t1', 'trash_file', 'f9', NULL, NULL, NULL),
               ('h1', 'hydrate_file', 'f1', NULL, NULL, NULL),
               ('p1', 'pin_tree', 'f1', NULL, NULL, NULL),
               ('d1', 'upload_version', 'f3', '<dir>/d1', 'w4', 'minted');
             {COMPLETED}"
        ));
        stage(dir.path(), &["s1", "s2", "c1", "d1"]);
        let counts = WaitingChanges::read(&db).unwrap().counts();
        assert_eq!(counts.files, 2, "one file per file, the completed upload not counted");
        assert_eq!(counts.others, 2, "the rename and the trash; never the hydrate or the pin");
    }

    /// U2, amended (spec §2 "Live", plan reconciliation row 18).
    #[test]
    fn parked_and_paused_writes_are_waiting_but_not_live() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin,
                                          attempts, max_attempts, paused_reason) VALUES
               ('parked', 'upload_version', 'f1', '<dir>/a', 'w1', 'minted', 25, 25, NULL),
               ('auth', 'upload_version', 'f2', '<dir>/b', 'w2', 'minted', 0, 25, 'auth'),
               ('held', 'upload_version', 'f3', '<dir>/c', 'w3', 'minted', 0, 25, 'key_replaced'),
               ('folder', 'create_folder', 'f4', NULL, NULL, NULL, 0, 5, 'quota');",
        );
        stage(dir.path(), &["a", "b", "c"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        let counts = waiting.counts();
        assert_eq!((counts.files, counts.others, counts.live), (3, 1, 0));
        let ready = SendConditions { engine_running: true, vault_unlocked: true, session_expired: false, link_failed: false, sync_paused: false };
        assert_eq!(send_readiness(ready, &waiting), Some(NotSending::Held), "can_send is false, and the reason is held");
    }

    /// U3.
    #[test]
    fn a_missing_staged_copy_counts_as_missing_never_as_nothing() {
        let (_dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('s1', 'upload_version', 'f1', '<dir>/gone', 'w1', 'minted');",
        );
        let counts = WaitingChanges::read(&db).unwrap().counts();
        assert_eq!((counts.files, counts.missing_files, counts.copied_files), (1, 1, 0));
    }

    /// U4: one `StateDb` call, one mutex hold, one transaction.
    #[test]
    fn waiting_changes_is_one_read() {
        let state_db = include_str!("../state_db.rs");
        let from = state_db.find("pub fn waiting_rows(").expect("waiting_rows");
        let body = &state_db[from..from + state_db[from..].find("\n    }\n").unwrap()];
        assert_eq!(body.matches(".lock()").count(), 1, "{body}");
        assert_eq!(body.matches("transaction(").count(), 1, "{body}");
        let this = include_str!("waiting.rs");
        let from = this.find("pub fn read(db: &StateDb)").expect("read");
        let read = &this[from..from + this[from..].find("\n    }\n").unwrap()];
        assert_eq!(read.matches("db.").count(), 1, "one StateDb call: {read}");
        assert!(read.contains("db.waiting_rows()"), "{read}");
    }

    /// U5: one row per condition, in the spec's order (§6.1).
    #[test]
    fn can_send_follows_each_condition() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('s1', 'upload_version', 'f1', '<dir>/a', 'w1', 'minted');",
        );
        stage(dir.path(), &["a"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        let good = SendConditions { engine_running: true, vault_unlocked: true, session_expired: false, link_failed: false, sync_paused: false };
        let cases = [
            (SendConditions { engine_running: false, ..good }, Some(NotSending::EngineStopped)),
            (SendConditions { vault_unlocked: false, ..good }, Some(NotSending::EngineStopped)),
            (SendConditions { session_expired: true, ..good }, Some(NotSending::Session)),
            (SendConditions { link_failed: true, ..good }, Some(NotSending::Offline)),
            (SendConditions { sync_paused: true, ..good }, Some(NotSending::Paused)),
            (good, None),
        ];
        for (conditions, expected) in cases {
            assert_eq!(send_readiness(conditions, &waiting), expected, "{conditions:?}");
        }
        assert_eq!(send_readiness(good, &WaitingChanges::default()), None, "nothing waits: nothing to send, no reason");
    }

    /// U15 (the count half; Task 5 adds the copy half).
    #[test]
    fn completed_and_released_payloads_are_never_sources() {
        let (dir, db) = db_with(&format!(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
             VALUES ('d1', 'upload_version', 'f3', '<dir>/d1', 'w4', 'minted');
             {COMPLETED}
             INSERT INTO staged_payloads (path, completed) VALUES ('<dir>/released', 1), ('<dir>/orphan', 0);"
        ));
        stage(dir.path(), &["d1", "released", "orphan"]);
        let waiting = WaitingChanges::read(&db).unwrap();
        assert!(waiting.is_empty(), "{waiting:?}");
    }

    /// U38 (the count half; Task 5 adds the keep half).
    #[test]
    fn copied_files_counts_files_with_a_copy() {
        let (dir, db) = db_with(
            "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
               ('x-old', 'upload_version', 'x', '<dir>/x-old', 'w1', 'earlier_build'),
               ('x-new', 'upload_version', 'x', '<dir>/x-new', 'w2', 'minted'),
               ('y', 'upload_version', 'y', '<dir>/y', 'w3', 'minted');",
        );
        stage(dir.path(), &["x-new"]);
        let counts = WaitingChanges::read(&db).unwrap().counts();
        assert_eq!((counts.files, counts.missing_files, counts.copied_files), (2, 2, 1));
    }

    /// §2 and §7.2: each write is exactly one of copied, covered, missing.
    #[test]
    fn classification_marks_copied_covered_and_missing() {
        use WriteState::*;
        let cases: [(&str, &[(&str, bool)], &[WriteState]); 5] = [
            ("two minted saves", &[("minted", true), ("minted", true)], &[Covered, Copied]),
            ("earlier build, then minted", &[("earlier_build", true), ("minted", true)], &[Copied, Copied]),
            ("newest copy gone", &[("minted", true), ("minted", false)], &[Copied, Missing]),
            ("earlier build gone", &[("earlier_build", false), ("minted", true)], &[Missing, Copied]),
            ("older minted gone", &[("minted", false), ("minted", true)], &[Covered, Copied]),
        ];
        for (name, writes, expected) in cases {
            let mut sql = String::from("INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES ");
            let rows: Vec<String> = writes
                .iter()
                .enumerate()
                .map(|(i, (origin, _))| format!("('op{i}', 'upload_version', 'f1', '<dir>/op{i}', 'w{i}', '{origin}')"))
                .collect();
            sql.push_str(&rows.join(", "));
            sql.push(';');
            let (dir, db) = db_with(&sql);
            for (i, (_, present)) in writes.iter().enumerate() {
                if *present {
                    stage(dir.path(), &[&format!("op{i}")]);
                }
            }
            let waiting = WaitingChanges::read(&db).unwrap();
            let states: Vec<WriteState> = waiting.files[0].writes.iter().map(|w| w.state).collect();
            assert_eq!(states, expected, "{name}");
        }
    }
}
```

- [ ] **Step 2: Run them, expect FAIL.**

Run: the test loop with `<filter>` = `sign_out::waiting`, `<N>` = 1, `<step>` = `red`.
Expected: compile errors for `waiting_rows`, `WaitingChanges` and the rest, or, once stubs exist, 8 tests FAILED on their counts.

- [ ] **Step 3: Implement.** In `state_db.rs`, after `pending_changes_count`:

```rust
/// One queued operation as a sign-out reads it (spec 2026-10-10 §2, W2): every op but a hydrate or a
/// pin, with its file's path and whether its upload has a recorded completion.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingRow {
    pub op_id: String,
    pub kind: OperationKind,
    pub file_id: Option<String>,
    pub file_path: Option<String>,
    pub payload_path: Option<String>,
    pub origin: Option<WriteOrigin>,
    /// The queue's order (`rowid`): higher was queued later.
    pub queue_order: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    pub paused: bool,
    /// `upload_resume.completed_version` is set: the server has these bytes (§8 item 3).
    pub completed: bool,
}
```

and in `impl StateDb`:

```rust
    /// Spec 2026-10-10 W2: the queue a sign-out counts, in ONE read (one mutex hold, one transaction), in queue
    /// order. Hydrates and pins are downloads, never counted.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn waiting_rows(&self) -> Result<Vec<WaitingRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT q.op_id, q.kind, q.file_id, f.path, q.payload_path, q.write_origin, q.rowid,
                        q.attempts, q.max_attempts, q.paused_reason IS NOT NULL,
                        EXISTS(SELECT 1 FROM upload_resume r
                               WHERE r.op_id = q.op_id AND r.completed_version IS NOT NULL)
                 FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind NOT IN ('hydrate_file', 'pin_tree')
                 ORDER BY q.rowid",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok(WaitingRow {
                    op_id: row.get(0)?,
                    kind: OperationKind::from_str(&row.get::<_, String>(1)?),
                    file_id: row.get(2)?,
                    file_path: row.get(3)?,
                    payload_path: row.get(4)?,
                    origin: WriteOrigin::from_db(row.get::<_, Option<String>>(5)?.as_deref()),
                    queue_order: row.get(6)?,
                    attempts: row.get(7)?,
                    max_attempts: row.get(8)?,
                    paused: row.get(9)?,
                    completed: row.get(10)?,
                })
            })?;
            mapped.collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(rows)
    }
```

Then `sign_out/waiting.rs` above its tests:

```rust
//! Spec 2026-10-10 §2 and W2 (task 1887): what waits in the queue, as one sign-out counts it. A waiting write is an
//! upload with no recorded completion; per file, each write is copied, covered or missing (§7.2). An other waiting
//! change is a folder create, rename, move, trash or restore. Every number a person sees comes from here.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::state_db::{OperationKind, StateDb, WaitingRow, WriteOrigin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteState {
    /// The keep step copies it (§2 "Copied").
    Copied,
    /// A newer copied write of the same file holds its bytes (§2 "Covered").
    Covered,
    /// Its staged copy is gone and nothing covers it (§2 "Missing").
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingWrite {
    pub op_id: String,
    pub payload_path: Option<PathBuf>,
    pub origin: WriteOrigin,
    pub queue_order: i64,
    pub live: bool,
    pub state: WriteState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingFile {
    pub file_id: String,
    /// `files.path` as the keep step names the copy (§7.2), or `None` (a bug: `Unnamed …`).
    pub path: Option<String>,
    /// Oldest first.
    pub writes: Vec<WaitingWrite>,
}

impl WaitingFile {
    pub fn has_copy(&self) -> bool {
        self.writes.iter().any(|write| write.state == WriteState::Copied)
    }

    pub fn is_missing(&self) -> bool {
        self.writes.iter().any(|write| write.state == WriteState::Missing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherChange {
    pub op_id: String,
    pub live: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaitingCounts {
    pub files: usize,
    pub copied_files: usize,
    pub others: usize,
    pub missing_files: usize,
    /// Live ops, writes and other changes.
    pub live: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WaitingChanges {
    pub files: Vec<WaitingFile>,
    pub others: Vec<OtherChange>,
}

fn is_upload(kind: &OperationKind) -> bool {
    matches!(kind, OperationKind::UploadFile | OperationKind::UploadVersion)
}

fn is_other_change(kind: &OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::CreateFolder
            | OperationKind::RenameFile
            | OperationKind::MoveFile
            | OperationKind::TrashFile
            | OperationKind::RestoreFile
            | OperationKind::RestoreVersion
    )
}

/// §2 "Missing": `metadata()` is `NotFound`, the test 1873 uses for `payload_missing`. Any other answer counts as
/// present, and a copy that then fails stops the sign-out (W8) instead of dropping the write.
pub fn staged_copy_exists(path: &Path) -> bool {
    !matches!(std::fs::metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

impl WaitingChanges {
    /// W2: one `StateDb` call, then the file system is asked which staged copies exist.
    pub fn read(db: &StateDb) -> Result<Self, String> {
        let rows = db.waiting_rows().map_err(|error| format!("read the queue: {error}"))?;
        Ok(Self::classify(rows, staged_copy_exists))
    }

    pub fn classify(rows: Vec<WaitingRow>, exists: impl Fn(&Path) -> bool) -> Self {
        let mut by_file: BTreeMap<String, WaitingFile> = BTreeMap::new();
        let mut others = Vec::new();
        for row in rows {
            let live = row.attempts < row.max_attempts && !row.paused;
            if is_upload(&row.kind) {
                if row.completed {
                    continue;
                }
                let key = row.file_id.clone().unwrap_or_else(|| row.op_id.clone());
                let present = row.payload_path.as_deref().is_some_and(|path| exists(Path::new(path)));
                let file = by_file.entry(key.clone()).or_insert_with(|| WaitingFile {
                    file_id: key,
                    path: row.file_path.clone(),
                    writes: Vec::new(),
                });
                file.writes.push(WaitingWrite {
                    op_id: row.op_id,
                    payload_path: row.payload_path.map(PathBuf::from),
                    // An op with no recorded origin is treated as an earlier build's: its copy is kept (§7.2).
                    origin: row.origin.unwrap_or(WriteOrigin::EarlierBuild),
                    queue_order: row.queue_order,
                    live,
                    state: if present { WriteState::Copied } else { WriteState::Missing },
                });
            } else if is_other_change(&row.kind) {
                others.push(OtherChange { op_id: row.op_id, live });
            }
        }
        let mut files: Vec<WaitingFile> = by_file.into_values().collect();
        for file in &mut files {
            settle_states(&mut file.writes);
        }
        Self { files, others }
    }

    pub fn counts(&self) -> WaitingCounts {
        WaitingCounts {
            files: self.files.len(),
            copied_files: self.files.iter().filter(|file| file.has_copy()).count(),
            others: self.others.len(),
            missing_files: self.files.iter().filter(|file| file.is_missing()).count(),
            live: self.files.iter().flat_map(|file| &file.writes).filter(|write| write.live).count()
                + self.others.iter().filter(|other| other.live).count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.others.is_empty()
    }

    /// The progress line's unit (§2 "Waiting change"): waiting files plus other waiting changes.
    pub fn changes(&self) -> usize {
        self.files.len() + self.others.len()
    }

    pub fn any_live(&self) -> bool {
        self.counts().live > 0
    }

    pub fn file_ids(&self) -> BTreeSet<String> {
        self.files.iter().map(|file| file.file_id.clone()).collect()
    }

    pub fn other_op_ids(&self) -> BTreeSet<String> {
        self.others.iter().map(|other| other.op_id.clone()).collect()
    }

    pub fn missing_op_ids(&self) -> BTreeSet<String> {
        self.files
            .iter()
            .flat_map(|file| &file.writes)
            .filter(|write| write.state == WriteState::Missing)
            .map(|write| write.op_id.clone())
            .collect()
    }
}

/// §2 and §7.2. On entry `state` says only whether the staged copy exists (`Copied`) or not (`Missing`). The newest
/// write with a copy is copied; so is every earlier-build write with a copy; a minted write older than the newest
/// copied one is covered; every other write is missing.
fn settle_states(writes: &mut [WaitingWrite]) {
    writes.sort_by_key(|write| write.queue_order);
    let newest_copied = writes
        .iter()
        .rev()
        .find(|write| write.state == WriteState::Copied)
        .map(|write| write.queue_order);
    for write in writes.iter_mut() {
        let present = write.state == WriteState::Copied;
        write.state = match newest_copied {
            Some(newest) if write.queue_order == newest => WriteState::Copied,
            _ if present && write.origin == WriteOrigin::EarlierBuild => WriteState::Copied,
            Some(newest) if write.origin == WriteOrigin::Minted && write.queue_order < newest => WriteState::Covered,
            _ => WriteState::Missing,
        };
    }
}

/// §6.1: why a sign-out would not send first, in the spec's order. `None` with something waiting: it can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotSending {
    EngineStopped,
    Session,
    Offline,
    Paused,
    Held,
}

/// §6.1 conditions 1-4; condition 5 (something is live) is read from the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendConditions {
    pub engine_running: bool,
    pub vault_unlocked: bool,
    pub session_expired: bool,
    pub link_failed: bool,
    pub sync_paused: bool,
}

/// `None`: nothing waits, or it can send. `Some(reason)`: the first condition that fails.
pub fn send_readiness(conditions: SendConditions, waiting: &WaitingChanges) -> Option<NotSending> {
    if waiting.is_empty() {
        return None;
    }
    if !conditions.engine_running || !conditions.vault_unlocked {
        Some(NotSending::EngineStopped)
    } else if conditions.session_expired {
        Some(NotSending::Session)
    } else if conditions.link_failed {
        Some(NotSending::Offline)
    } else if conditions.sync_paused {
        Some(NotSending::Paused)
    } else if !waiting.any_live() {
        Some(NotSending::Held)
    } else {
        None
    }
}
```

- [ ] **Step 4: Run, expect PASS.** The test loop, filter `sign_out::waiting`, step `green`. Expected: `ok. 8 passed; 0 failed`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. In `classify`, push one `WaitingFile` per write instead of grouping (`by_file.entry(row.op_id.clone())`): U1 red on `counts.files` (3).
  2. Remove `'hydrate_file', 'pin_tree'` from the `WHERE`, and treat every non-upload as an other change: U1 red on `counts.others` (4).
  3. Remove `if row.completed { continue; }`: U1 red on `counts.files` (3), U15 red on `is_empty`.
  4. `let live = row.attempts < row.max_attempts;` (drop the pause): U2 red on `counts.live`.
  5. Skip rows whose copy is missing (`if !present { continue; }` before the push): U3 red (`files = 0`).
  6. Add a second `let _ = self.0.lock()` read in `waiting_rows`: U4 red.
  7. Swap the `session_expired` and `link_failed` branches: U5 red on the `Session` row.
  8. Compute `copied_files` as `files - missing_files`: U38 red (0).
  9. Drop the `EarlierBuild` arm of `settle_states`: the classification test red on "earlier build, then minted".

- [ ] **Step 6: The task gate** with `<N>` = 1 and `<L>` = `L0 + 8`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/waiting.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
git commit -m "feat(macos): count what waits in the queue in one read, for the sign-out (1887)" -- src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/waiting.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
```

### Task 2: Kept names and roots

**Files:**
- Create: `src-tauri/src/sign_out/kept_names.rs`
- Modify: `src-tauri/src/sign_out/mod.rs` (add `pub(crate) mod kept_names;`)
- Modify: `src-tauri/src/lib.rs:4000-4029` (`disposable_cache_roots` becomes a wrapper over `disposable_cache_roots_in`)
- Test: `src-tauri/src/sign_out/kept_names.rs` (pure), and a new `#[cfg(all(test, target_os = "macos"))] mod sign_out_keep_tests` at the end of `lib.rs` (U12, which reads private `lib.rs` items)

**Interfaces:**
- Consumes: `engine_bridge::FinderStagingBases { data, cache, temp }` and `.candidates()` (`@184:engine_bridge.rs:6235-6301`); `staged_payload::sweep_unreferenced` (`@184:staged_payload.rs:197`).
- Produces:
  - `kept_names::{KEPT_FOLDER_NAME, APP_BUNDLE_ID, MAX_NAME_BYTES, KeptRole, kept_root, kept_root_for_this_app, kept_relative_path, with_suffix, cut_name, event_folder_name}`.
  - `kept_root(document_dir: Option<&Path>, data_dir: Option<&Path>, bundle_id: &str) -> Option<PathBuf>`.
  - `kept_relative_path(file_path: Option<&str>, op_id: &str, role: KeptRole) -> Vec<String>` (components; the last is the file name).
  - `with_suffix(name: &str, suffix: &str) -> String` (the suffix goes before the extension; the stem is cut so the whole name is at most 255 bytes).
  - `event_folder_name(at: chrono::DateTime<chrono::Local>) -> String`.
  - `lib.rs`: `fn disposable_cache_roots_in(temp_dir: PathBuf, cache_dir: Option<PathBuf>, hydrate_cache_dir: Option<PathBuf>, staging: &engine_bridge::FinderStagingBases) -> Vec<PathBuf>`.

- [ ] **Step 1: Write the failing tests.** In `kept_names.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// U33.
    #[test]
    fn kept_root_is_the_container_or_app_support() {
        let data = Path::new("/Users/a/Library/Containers/io.beebeeb.app/Data/Library/Application Support");
        let sandboxed = Path::new("/Users/a/Library/Containers/io.beebeeb.app/Data/Documents");
        assert_eq!(
            kept_root(Some(sandboxed), Some(data), APP_BUNDLE_ID),
            Some(sandboxed.join("Changes not sent"))
        );
        let fallback = Some(data.join("beebeeb").join("Changes not sent"));
        assert_eq!(kept_root(Some(Path::new("/Users/a/Documents")), Some(data), APP_BUNDLE_ID), fallback, "unsandboxed");
        let other = Path::new("/Users/a/Library/Containers/com.example.other/Data/Documents");
        assert_eq!(kept_root(Some(other), Some(data), APP_BUNDLE_ID), fallback, "another bundle's container");
        assert_eq!(kept_root(None, Some(data), APP_BUNDLE_ID), fallback);
        assert_eq!(kept_root(None, None, APP_BUNDLE_ID), None);
    }

    /// The bundle id the root is checked against is the app's own (`tauri.conf.json`).
    #[test]
    fn the_bundle_id_is_the_apps() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"].as_str(), Some(APP_BUNDLE_ID));
    }

    /// U11, the naming half (Task 3 has the disk half).
    #[test]
    fn kept_names_follow_the_rule() {
        let name = |path: Option<&str>, role| kept_relative_path(path, "0123456789abcdef", role);
        assert_eq!(name(Some("Work/report.docx"), KeptRole::Newest), ["Work", "report.docx"]);
        assert_eq!(name(Some("/Work//report.docx"), KeptRole::Newest), ["Work", "report.docx"], "empty components dropped");
        assert_eq!(
            name(Some("Old/a.txt"), KeptRole::EarlierSave(1)),
            ["Old", "a (earlier save 1).txt"]
        );
        assert_eq!(name(Some("Old/a.txt"), KeptRole::EarlierSave(2)), ["Old", "a (earlier save 2).txt"]);
        assert_eq!(name(Some(".env"), KeptRole::EarlierSave(1)), [".env (earlier save 1)"], "a dotfile has no extension");
        assert_eq!(name(Some("a/../b/./c"), KeptRole::Newest), ["a", "_", "b", "_", "c"]);
        assert_eq!(name(Some("a/b\0c"), KeptRole::Newest), ["a", "_"]);
        assert_eq!(name(None, KeptRole::Newest), ["Unnamed 01234567"]);
        assert_eq!(name(Some("/"), KeptRole::Newest), ["Unnamed 01234567"]);

        let long = format!("{}.docx", "x".repeat(300));
        let cut = name(Some(&long), KeptRole::Newest).pop().unwrap();
        assert_eq!(cut.len(), 255, "{cut}");
        assert!(cut.ends_with(".docx"));
        let accented = format!("{}.txt", "é".repeat(200)); // 2 bytes each
        let cut = name(Some(&accented), KeptRole::Newest).pop().unwrap();
        assert!(cut.len() <= 255 && cut.ends_with(".txt") && cut.is_char_boundary(cut.len() - 4), "{cut}");

        let full = format!("{}.docx", "y".repeat(250)); // exactly 255 bytes
        let with = with_suffix(&full, " (2)");
        assert_eq!(with.len(), 255);
        assert!(with.ends_with(" (2).docx"), "{with}");
    }

    #[test]
    fn the_event_folder_reads_like_a_screenshot() {
        use chrono::TimeZone;
        let at = chrono::Local.with_ymd_and_hms(2026, 10, 10, 14, 5, 12).unwrap();
        assert_eq!(event_folder_name(at), "2026-10-10 at 14.05.12");
    }
}
```

In `lib.rs`, at the end of the file:

```rust
/// Spec 2026-10-10 (task 1887): the sign-out's keep step and its purge, on a Mac.
#[cfg(all(test, target_os = "macos"))]
mod sign_out_keep_tests {
    use super::*;
    use crate::engine_bridge::FinderStagingBases;
    use crate::sign_out::kept_names::{APP_BUNDLE_ID, kept_root};
    use std::path::Path;

    /// U12 on the release layout (spec §7.1, amended for `184ac962`).
    #[test]
    fn the_kept_folder_is_out_of_every_purges_reach() {
        let container = Path::new("/Users/a/Library/Containers/io.beebeeb.app/Data");
        let data = container.join("Library").join("Application Support");
        let cache = container.join("Library").join("Caches");
        let staging = FinderStagingBases { data: Some(data.clone()), cache: Some(cache.clone()), temp: container.join("tmp") };
        let hydrate = PathBuf::from("/Users/a/Library/Group Containers/R8352WDJJR.io.beebeeb.app.fileprovider/hydrate");
        let roots = disposable_cache_roots_in(container.join("tmp"), Some(cache.clone()), Some(hydrate), &staging);
        let staging_list = staging.candidates();
        let kept = [
            kept_root(Some(&container.join("Documents")), Some(&data), APP_BUNDLE_ID).unwrap(),
            kept_root(Some(Path::new("/Users/a/Documents")), Some(&data), APP_BUNDLE_ID).unwrap(),
        ];
        for kept in &kept {
            for root in roots.iter().chain(staging_list.iter()) {
                assert!(!kept.starts_with(root), "{} is under {}", kept.display(), root.display());
            }
        }
        let durable = data.join("beebeeb").join("finder-writes");
        assert!(roots.contains(&durable), "the purge's allow-list holds the durable staging root");
        assert!(staging_list.contains(&durable), "the orphan sweep holds it too");
        assert!(!roots.contains(&data) && !staging_list.contains(&data), "never the whole data dir");
        assert!(staging_list.contains(&cache.join("beebeeb").join("finder-writes")), "the old Caches root stays");

        let production = super::finder_setup_command_tests::production_source();
        let wrapper = super::finder_setup_command_tests::body_between(&production, "fn disposable_cache_roots() -> Vec<PathBuf> {", "\n}\n");
        assert_eq!(wrapper.matches("disposable_cache_roots_in(").count(), 1, "{wrapper}");
        assert!(!wrapper.contains("roots.push") && !wrapper.contains("vec!["), "the wrapper builds nothing itself: {wrapper}");
    }

    /// U12's last clause: neither engine-start step touches a kept file.
    #[test]
    fn the_engine_start_sweep_never_lists_a_kept_file() {
        let dir = tempfile::tempdir().unwrap();
        let container = dir.path().join("Data");
        let staging = FinderStagingBases {
            data: Some(container.join("Library/Application Support")),
            cache: Some(container.join("Library/Caches")),
            temp: container.join("tmp"),
        };
        for folder in staging.candidates() {
            std::fs::create_dir_all(&folder).unwrap();
        }
        let kept = container.join("Documents/Changes not sent/2026-10-10 at 14.05.12/Work");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::write(kept.join("report.docx"), b"kept").unwrap();
        let db = state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        crate::staged_payload::sweep_unreferenced(&db, &staging.candidates()).unwrap();
        let repair = db.engine_start_repair().unwrap();
        assert!(repair.released_payloads.is_empty());
        assert_eq!(std::fs::read(kept.join("report.docx")).unwrap(), b"kept");
    }
}
```

- [ ] **Step 2: Run them, expect FAIL.** The test loop with filters `sign_out::kept_names` and then `sign_out_keep_tests`. Expected: compile errors (`kept_root`, `disposable_cache_roots_in` missing).

- [ ] **Step 3: Implement.** `src-tauri/src/sign_out/kept_names.rs` above its tests:

```rust
//! Spec 2026-10-10 §7.1 and §7.2 (task 1887): where the kept folder is, and what each kept file is called.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::path::{Path, PathBuf};

/// The folder's name under the container's `Documents` (§7.1).
pub const KEPT_FOLDER_NAME: &str = "Changes not sent";
/// `tauri.conf.json`'s `identifier`; a test pins the two equal.
pub const APP_BUNDLE_ID: &str = "io.beebeeb.app";
/// The longest file name APFS takes, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// §7.2: the newest copied write of a file gets the plain name; every other copied write (an earlier build's) is
/// `<stem> (earlier save <N>)<.ext>`, N = 1 for the next-newest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptRole {
    Newest,
    EarlierSave(u32),
}

/// §7.1 (review M5): `<document_dir>/Changes not sent` only inside this app's sandbox container, where
/// `document_dir` is `<home>/Library/Containers/<bundle_id>/Data/Documents`; otherwise
/// `<data_dir>/beebeeb/Changes not sent`, so an unsandboxed build never writes plaintext into the person's real
/// Documents folder.
pub fn kept_root(document_dir: Option<&Path>, data_dir: Option<&Path>, bundle_id: &str) -> Option<PathBuf> {
    let container_documents = Path::new("Library").join("Containers").join(bundle_id).join("Data").join("Documents");
    match document_dir {
        Some(dir) if dir.is_absolute() && dir.ends_with(&container_documents) => Some(dir.join(KEPT_FOLDER_NAME)),
        _ => data_dir.map(|data| data.join("beebeeb").join(KEPT_FOLDER_NAME)),
    }
}

/// [`kept_root`] for this process.
pub fn kept_root_for_this_app() -> Option<PathBuf> {
    kept_root(dirs::document_dir().as_deref(), dirs::data_dir().as_deref(), APP_BUNDLE_ID)
}

/// §7.1: local time at the start of the keep step, as macOS names screenshots (dots, never colons).
pub fn event_folder_name(at: chrono::DateTime<chrono::Local>) -> String {
    at.format("%Y-%m-%d at %H.%M.%S").to_string()
}

/// `name` split before its extension. A name that starts with a dot and has no other dot has none.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(0) | None => (name, ""),
        Some(dot) => (&name[..dot], &name[dot..]),
    }
}

/// The longest prefix of `text` that fits in `max` bytes, cut at a character boundary.
fn prefix_within(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `name` cut to 255 bytes; the stem is cut, never the extension.
pub fn cut_name(name: &str) -> String {
    with_suffix(name, "")
}

/// `<stem><suffix><.ext>`, the stem cut so the whole is at most 255 bytes (§7.2 "Lengths": the cut applies to the
/// final name, never to the suffix or the extension).
pub fn with_suffix(name: &str, suffix: &str) -> String {
    let (stem, extension) = split_extension(name);
    let room = MAX_NAME_BYTES.saturating_sub(suffix.len() + extension.len());
    format!("{}{suffix}{extension}", prefix_within(stem, room))
}

/// §7.2 "Names": the components of a kept file's path under the event folder.
pub fn kept_relative_path(file_path: Option<&str>, op_id: &str, role: KeptRole) -> Vec<String> {
    let mut components: Vec<String> = file_path
        .unwrap_or_default()
        .split('/')
        .filter(|component| !component.is_empty())
        .map(|component| {
            if component == "." || component == ".." || component.contains('\0') {
                "_".to_string()
            } else {
                component.to_string()
            }
        })
        .collect();
    if components.is_empty() {
        components.push(format!("Unnamed {}", prefix_within(op_id, 8)));
    }
    let last = components.len() - 1;
    for (index, component) in components.iter_mut().enumerate() {
        *component = if index < last {
            prefix_within(component, MAX_NAME_BYTES).to_string()
        } else {
            match role {
                KeptRole::Newest => cut_name(component),
                KeptRole::EarlierSave(n) => with_suffix(component, &format!(" (earlier save {n})")),
            }
        };
    }
    components
}
```

Add `pub(crate) mod kept_names;` to `sign_out/mod.rs`. In `lib.rs`, replace the body of `disposable_cache_roots` (`@184:lib.rs:4000-4029`) with the core and a one-line wrapper; move its comments onto the core unchanged:

```rust
fn disposable_cache_roots() -> Vec<PathBuf> {
    disposable_cache_roots_in(std::env::temp_dir(), dirs::cache_dir(), macos_hydrate_cache_root(), &engine_bridge::FinderStagingBases::current())
}

/// The hydrate cache's root on a Mac (task 1670 round 2, Codex P1); none elsewhere.
fn macos_hydrate_cache_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::ipc_socket::macos_hydrate_cache_dir())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Spec 2026-10-10 §7.1 (U12): the roots `is_disposable_cache_path` accepts, from directories passed in, so a test can
/// check a release layout. The temp dir, the cache dir, the macOS hydrate cache, then every staging folder (spec
/// 2026-10-09 §8.4), never the whole data dir.
fn disposable_cache_roots_in(
    temp_dir: PathBuf,
    cache_dir: Option<PathBuf>,
    hydrate_cache_dir: Option<PathBuf>,
    staging: &engine_bridge::FinderStagingBases,
) -> Vec<PathBuf> {
    let mut roots = vec![temp_dir];
    roots.extend(cache_dir);
    roots.extend(hydrate_cache_dir);
    roots.extend(staging.candidates());
    roots
}
```

- [ ] **Step 4: Run, expect PASS.** Filters `sign_out::kept_names` (`ok. 4 passed`) and `sign_out_keep_tests` (`ok. 2 passed`). Also run the existing `disposable_cache_roots_includes_the_macos_hydrate_cache_dir` (`@184:lib.rs:15923`): still `ok. 1 passed`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `kept_root` returns `document_dir.map(|d| d.join(KEPT_FOLDER_NAME))` first: U33 red on "unsandboxed".
  2. The fallback becomes `data.join("beebeeb").join("finder-writes").join(KEPT_FOLDER_NAME)`: U12 red ("is under").
  3. `roots.push(staging.data.clone().unwrap())` in the core: U12 red on "never the whole data dir".
  4. Drop `roots.extend(staging.candidates())`: U12 red on "the purge's allow-list holds the durable staging root".
  5. Build the list inside `disposable_cache_roots()` again: U12 red on the wrapper pin.
  6. `with_suffix` cuts after adding the suffix (`prefix_within(&format!(...), 255)`): U11 red on `ends_with(" (2).docx")`.
  7. `split_extension` returns `Some(0)` as an extension split: U11 red on `.env (earlier save 1)`.

- [ ] **Step 6: The task gate** with `<N>` = 2 and `<L>` = `L0 + 14`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/kept_names.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
git commit -m "feat(macos): where kept changes go and what they are called (1887)" -- src-tauri/src/sign_out/kept_names.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
```

### Task 3: The kept-folder writer

**Files:**
- Create: `src-tauri/src/sign_out/kept_writer.rs` (macOS only)
- Modify: `src-tauri/src/sign_out/mod.rs` (add `#[cfg(target_os = "macos")] pub(crate) mod kept_writer;`)
- Test: `src-tauri/src/sign_out/kept_writer.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `kept_names::{with_suffix, MAX_NAME_BYTES}` (Task 2); `libc::clonefile`, `libc::removexattr`, `libc::XATTR_NOFOLLOW`.
- Produces:
  - `pub(crate) const CLONE_NOFOLLOW: u32 = 0x0001;` and `pub(crate) const PATH_MAX_BYTES: usize = 1024;`
  - `pub(crate) trait CloneFile { fn clone_file(&self, src: &Path, dst: &Path) -> std::io::Result<()>; }`, `pub(crate) struct SystemClone;` (the production clone with its fallback).
  - `pub(crate) struct KeptWriter<'a>`: `create(root: &Path, event_name: &str, clone: &'a dyn CloneFile) -> io::Result<Self>`, `event(&self) -> &Path`, `keep(&mut self, components: &[String], source: &Path) -> io::Result<PathBuf>`, `finish(self) -> io::Result<KeptFolder>`, `abandon(self)`.
  - `pub(crate) struct KeptFolder { pub path: PathBuf, pub files: usize }`.

- [ ] **Step 1: Write the failing tests** in `kept_writer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha(path: &Path) -> Vec<u8> {
        Sha256::digest(std::fs::read(path).unwrap()).to_vec()
    }

    fn staged(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join("staging").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn parts(path: &str) -> Vec<String> {
        crate::sign_out::kept_names::kept_relative_path(Some(path), "op", crate::sign_out::kept_names::KeptRole::Newest)
    }

    /// The disk rules need a case- and normalization-insensitive volume, as the default APFS is. Fail, never skip.
    fn assert_case_insensitive(dir: &Path) {
        std::fs::write(dir.join("a"), b"").unwrap();
        assert!(dir.join("A").exists(), "this test needs a case-insensitive volume for its temporary directory");
        std::fs::remove_file(dir.join("a")).unwrap();
    }

    /// U11, the disk half.
    #[test]
    fn kept_names_follow_the_rule_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert_case_insensitive(dir.path());
        let root = dir.path().join("Changes not sent");
        let mut writer = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let cases: [(&str, &str); 4] = [
            ("Work/Report.docx", "Work/report.docx"),
            ("Caf\u{e9}.txt", "Cafe\u{301}.txt"),
            ("Notes", "notes/a.txt"),
            ("dir-first/a.txt", "Dir-First"),
        ];
        for (index, (first, second)) in cases.iter().enumerate() {
            let a = staged(dir.path(), &format!("{index}-a"), format!("first {index}").as_bytes());
            let b = staged(dir.path(), &format!("{index}-b"), format!("second {index}").as_bytes());
            let kept_a = writer.keep(&parts(first), &a).unwrap();
            let kept_b = writer.keep(&parts(second), &b).unwrap();
            assert_ne!(kept_a, kept_b, "{first} / {second}");
            assert_eq!(sha(&kept_a), sha(&a), "{first}");
            assert_eq!(sha(&kept_b), sha(&b), "{second}");
        }
        let event = writer.event().to_path_buf();
        assert!(event.join("Work").join("report (2).docx").exists(), "the case-only twin");
        // File first: the directory takes " (2)", and a later file of that directory lands beside the first.
        assert!(event.join("notes (2)").join("a.txt").exists());
        let b2 = staged(dir.path(), "notes-b", b"b");
        assert_eq!(writer.keep(&parts("notes/b.txt"), &b2).unwrap(), event.join("notes (2)").join("b.txt"));
        // Directory first: the file takes " (2)".
        assert!(event.join("Dir-First (2)").is_file());
        // A 255-byte name that collides keeps its " (2)" and its extension.
        let long = format!("{}.docx", "y".repeat(250));
        let one = staged(dir.path(), "long-1", b"1");
        let two = staged(dir.path(), "long-2", b"2");
        writer.keep(&parts(&long), &one).unwrap();
        let second = writer.keep(&parts(&long), &two).unwrap();
        let name = second.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 255);
        assert!(name.ends_with(" (2).docx"), "{name}");
        let kept = writer.finish().unwrap();
        assert_eq!(kept.files, 11);
    }

    /// `event/<path>` exactly `total` bytes long, from components of at most 200 bytes.
    fn padded(event: &Path, total: usize, tag: char) -> String {
        let mut remaining = total - event.as_os_str().len() - 1;
        let mut parts = Vec::new();
        while remaining > 205 {
            parts.push(tag.to_string().repeat(200));
            remaining -= 201;
        }
        parts.push(format!("{}.bin", tag.to_string().repeat(remaining - 4)));
        parts.join("/")
    }

    /// U11, path length (re-review 2 Q2): 1023 bytes in place, 1024 at the top, 1022 + " (2)" at the top.
    #[test]
    fn long_paths_go_to_the_top_of_the_event_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Changes not sent");
        let mut writer = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let event = writer.event().to_path_buf();
        let fits = padded(&event, 1023, 'a');
        let too_long = padded(&event, 1024, 'b');
        let grows = padded(&event, 1022, 'c');
        let source = |name: &str| staged(dir.path(), name, name.as_bytes());
        let kept = writer.keep(&parts(&fits), &source("fits")).unwrap();
        assert_eq!(kept, event.join(&fits), "1023 bytes is made in place");
        assert_eq!(kept.as_os_str().len(), 1023);
        let kept = writer.keep(&parts(&too_long), &source("too-long")).unwrap();
        assert_eq!(kept.parent().unwrap(), event, "1024 bytes goes to the top");
        writer.keep(&parts(&grows), &source("grows-1")).unwrap();
        let kept = writer.keep(&parts(&grows), &source("grows-2")).unwrap();
        assert_eq!(kept.parent().unwrap(), event, "1022 bytes plus \" (2)\" goes to the top");
        assert_eq!(sha(&kept), sha(&dir.path().join("staging").join("grows-2")));
        writer.finish().unwrap();
    }

    /// U28.
    #[test]
    fn kept_copies_carry_no_backup_exclusion() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let source = staged(dir.path(), "excluded", b"bytes");
        let path = CString::new(source.as_os_str().as_bytes()).unwrap();
        let name = CString::new(BACKUP_EXCLUSION_ATTRIBUTE).unwrap();
        let value = b"com.apple.backupd";
        // SAFETY: both strings are NUL-terminated and outlive the call; `value` is a valid buffer of its length.
        let set = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
        assert_eq!(set, 0, "set the attribute on the staged copy");
        let mut writer = KeptWriter::create(&dir.path().join("kept"), "event", &SystemClone).unwrap();
        let kept = writer.keep(&parts("a.txt"), &source).unwrap();
        let kept_c = CString::new(kept.as_os_str().as_bytes()).unwrap();
        // SAFETY: as above; a null buffer asks only for the size.
        let size = unsafe { libc::getxattr(kept_c.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
        assert_eq!(size, -1, "the kept copy has no backup exclusion");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ENOATTR));
    }

    /// U40.
    #[test]
    fn clone_nofollow_is_the_headers_value() {
        assert_eq!(CLONE_NOFOLLOW, 0x0001, "sys/clonefile.h:33");
        let source = include_str!("kept_writer.rs");
        let call = &source[source.find("libc::clonefile(").expect("the clone call")..];
        let call = &call[..call.find(')').unwrap()];
        assert!(call.contains("CLONE_NOFOLLOW"), "{call}");
    }

    /// Review Focus 2: a keep right after a failed one, in the same second, gets a folder of its own.
    #[test]
    fn two_keeps_in_the_same_second_make_two_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Changes not sent");
        let first = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let first_event = first.event().to_path_buf();
        let second = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        assert_eq!(second.event(), root.join("2026-10-10 at 14.05.12 (2)"), "never split at the dots");
        first.abandon();
        assert!(!first_event.exists(), "a failed keep removes its half-made folder");
        let third = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        assert_eq!(third.event(), first_event, "the name is free again, and the folder is new and empty");
        assert_eq!(std::fs::read_dir(third.event()).unwrap().count(), 0);
    }
}
```

`sha2` 0.10 is already a dependency (`src-tauri/Cargo.toml:133`, `[dependencies]`). No crate is added.

- [ ] **Step 2: Run them, expect FAIL.** The test loop, filter `sign_out::kept_writer`. Expected: compile errors (`KeptWriter` missing).

- [ ] **Step 3: Implement** `kept_writer.rs` above its tests:

```rust
//! Spec 2026-10-10 §7.2 "How" (task 1887, macOS): copy a staged copy into the kept folder. Nothing can overwrite:
//! every destination is made exclusively, and the volume, not a string comparison, decides a collision.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use super::kept_names::{MAX_NAME_BYTES, with_suffix};

/// `sys/clonefile.h:33`. `libc` 0.2.189 has `clonefile` but not this flag (re-review N5).
pub(crate) const CLONE_NOFOLLOW: u32 = 0x0001;
/// `sys/syslimits.h:103`, counting the terminating NUL: a path of 1024 bytes or more cannot be made.
pub(crate) const PATH_MAX_BYTES: usize = 1024;
/// The attribute that keeps an item out of Time Machine. A kept file is the person's, so it is backed up.
pub(crate) const BACKUP_EXCLUSION_ATTRIBUTE: &str = "com.apple.metadata:com_apple_backup_excludeItem";

pub(crate) trait CloneFile {
    /// Make `dst` as a copy of `src`. Never replaces an existing `dst`: that is `AlreadyExists`.
    fn clone_file(&self, src: &Path, dst: &Path) -> io::Result<()>;
}

/// The production copy: an APFS clone, or, where the volume cannot clone, a new file and a byte copy.
pub(crate) struct SystemClone;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "a path holds a NUL"))
}

impl CloneFile for SystemClone {
    fn clone_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let (src_c, dst_c) = (c_path(src)?, c_path(dst)?);
        // SAFETY: both pointers are NUL-terminated C strings that live until the call returns.
        let rc = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), CLONE_NOFOLLOW) };
        if rc == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ENOTSUP) | Some(libc::EXDEV) => copy_new(src, dst),
            _ => Err(error),
        }
    }
}

/// The fallback: `create_new` (never truncates), the bytes, then the staged copy's modification time.
fn copy_new(src: &Path, dst: &Path) -> io::Result<()> {
    let mut from = File::open(src)?;
    let mut to = OpenOptions::new().write(true).create_new(true).open(dst)?;
    io::copy(&mut from, &mut to)?;
    to.set_modified(from.metadata()?.modified()?)?;
    Ok(())
}

fn remove_backup_exclusion(path: &Path) -> io::Result<()> {
    let (path_c, name_c) = (c_path(path)?, CString::new(BACKUP_EXCLUSION_ATTRIBUTE).expect("a constant without NUL"));
    // SAFETY: both pointers are NUL-terminated C strings that live until the call returns.
    let rc = unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr(), libc::XATTR_NOFOLLOW) };
    let error = io::Error::last_os_error();
    if rc == 0 || error.raw_os_error() == Some(libc::ENOATTR) { Ok(()) } else { Err(error) }
}

fn too_long(path: &Path) -> bool {
    path.as_os_str().len() >= PATH_MAX_BYTES
}

/// `name`, then `name (2)`, `name (3)`…, cut to 255 bytes. Directories are never split at a dot.
fn directory_candidate(name: &str, n: u32) -> String {
    let suffix = if n == 1 { String::new() } else { format!(" ({n})") };
    let mut stem = name.to_string();
    while stem.len() + suffix.len() > MAX_NAME_BYTES {
        stem.pop();
    }
    format!("{stem}{suffix}")
}

/// One event folder being written.
pub(crate) struct KeptWriter<'a> {
    root: PathBuf,
    event: PathBuf,
    clone: &'a dyn CloneFile,
    /// The vault directory (its components) → the directory this keep made or merged into for it.
    directories: HashMap<Vec<String>, PathBuf>,
    made: Vec<PathBuf>,
    files: usize,
}

pub(crate) struct KeptFolder {
    pub path: PathBuf,
    pub files: usize,
}

impl<'a> KeptWriter<'a> {
    /// Make `<root>/<event_name>`, or `… (2)`, `… (3)` when the name is taken (§7.1). The folder is new and empty.
    pub(crate) fn create(root: &Path, event_name: &str, clone: &'a dyn CloneFile) -> io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let mut n = 1;
        let event = loop {
            let candidate = root.join(directory_candidate(event_name, n));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(error) => return Err(error),
            }
        };
        Ok(Self { root: root.to_path_buf(), event, clone, directories: HashMap::new(), made: Vec::new(), files: 0 })
    }

    pub(crate) fn event(&self) -> &Path {
        &self.event
    }

    /// The directory on disk for the vault directory `components`, made one component at a time (never
    /// `create_dir_all`, which fails when a file holds the name). A directory already there is merged into (a
    /// case-only or normalization-only twin); a file there gives the next " (n)".
    fn directory_for(&mut self, components: &[String]) -> io::Result<PathBuf> {
        let mut parent = self.event.clone();
        for depth in 0..components.len() {
            let key = components[..=depth].to_vec();
            if let Some(known) = self.directories.get(&key) {
                parent = known.clone();
                continue;
            }
            let mut n = 1;
            let made = loop {
                let candidate = parent.join(directory_candidate(&components[depth], n));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => {
                        self.made.push(candidate.clone());
                        break candidate;
                    }
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                        let there = std::fs::symlink_metadata(&candidate)?;
                        if there.is_dir() {
                            break candidate;
                        }
                        n += 1;
                    }
                    Err(error) => return Err(error),
                }
            };
            self.directories.insert(key, made.clone());
            parent = made;
        }
        Ok(parent)
    }

    /// Copy `source` to the vault path `components` (the last is the file name) and flush it. A destination of 1024
    /// bytes or more, planned or after any " (n)", goes to the top of the event folder under its last component.
    pub(crate) fn keep(&mut self, components: &[String], source: &Path) -> io::Result<PathBuf> {
        let name = components.last().expect("kept_relative_path never returns an empty path").clone();
        let planned = components.iter().fold(self.event.clone(), |path, part| path.join(part));
        let mut directory = if too_long(&planned) {
            self.event.clone()
        } else {
            self.directory_for(&components[..components.len() - 1])?
        };
        let mut n = 1;
        let kept = loop {
            let file_name = if n == 1 { name.clone() } else { with_suffix(&name, &format!(" ({n})")) };
            let destination = directory.join(&file_name);
            if too_long(&destination) && directory != self.event {
                directory = self.event.clone();
                n = 1;
                continue;
            }
            match self.clone.clone_file(source, &destination) {
                Ok(()) => break destination,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(error) => return Err(error),
            }
        };
        remove_backup_exclusion(&kept)?;
        File::open(&kept)?.sync_all()?;
        self.files += 1;
        Ok(kept)
    }

    /// Flush every directory this keep made, the event folder and the root, then hand back the folder.
    pub(crate) fn finish(self) -> io::Result<KeptFolder> {
        for directory in self.made.iter().rev().chain([&self.event, &self.root]) {
            File::open(directory)?.sync_all()?;
        }
        Ok(KeptFolder { path: self.event, files: self.files })
    }

    /// A failed keep (W8): the half-made folder holds only copies of bytes still in the queue, so it goes.
    pub(crate) fn abandon(self) {
        if let Err(error) = std::fs::remove_dir_all(&self.event) {
            tracing::warn!(kind = %error.kind(), "could not remove a half-made kept folder");
        }
    }
}
```

Add `#[cfg(target_os = "macos")] pub(crate) mod kept_writer;` to `sign_out/mod.rs`.

- [ ] **Step 4: Run, expect PASS.** Filter `sign_out::kept_writer`. Expected: `ok. 5 passed; 0 failed`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `SystemClone::clone_file` becomes `std::fs::copy(src, dst).map(|_| ())`: `kept_names_follow_the_rule_on_disk` red on a collision pair's sha (the second overwrote the first).
  2. `copy_new` with `.create(true).truncate(true)` and the clone call removed: the same red.
  3. `directory_for` uses `std::fs::create_dir_all(&candidate)`: the file-first case fails (`notes (2)` missing, or an error).
  4. Cut after adding the suffix (`format!("{name} ({n})")` then `cut_name`): red on `ends_with(" (2).docx")`.
  5. `too_long` compares `> PATH_MAX_BYTES`: `long_paths…` red on the 1024-byte case (`ENAMETOOLONG`).
  6. Check the length only on `planned`: red on the 1022 + " (2)" case.
  7. `CLONE_NOFOLLOW = 0x0008`: `clone_nofollow_is_the_headers_value` red.
  8. Drop `remove_backup_exclusion`: U28 red (`size` is not -1).
  9. `create` splits at the dots (`with_suffix(event_name, …)`): `two_keeps…` red on `"… (2)"`.

- [ ] **Step 6: The task gate** with `<N>` = 3 and `<L>` = `L0 + 19`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/kept_writer.rs src-tauri/src/sign_out/mod.rs
git commit -m "feat(macos): copy waiting saves into a kept folder with no overwrite (1887)" -- src-tauri/src/sign_out/kept_writer.rs src-tauri/src/sign_out/mod.rs
```

### Task 4: The two config keys and the durable save

**Files:**
- Modify: `src-tauri/src/config.rs` (two fields after `kept_unsynced_folder`, `@184:config.rs:278-284`, and their defaults at `:376`; `ConfigFs`, `RealConfigFs`, `save_to_durable`, `update_at_durable`, `update_at_durable_with` next to `try_update_at_then`, `@184:config.rs:806-843`)
- Create: `src-tauri/src/sign_out/kept_record.rs`
- Modify: `src-tauri/src/sign_out/mod.rs` (add `pub(crate) mod kept_record;`)
- Test: `src-tauri/src/sign_out/kept_record.rs`

**Interfaces:**
- Consumes: `DesktopConfig::{update_at, load_from, apply_settings}`, `config::DesktopSettings`, `finder_removal::{DismissOutcome, remember_kept_folder_at}`.
- Produces:
  - `DesktopConfig.kept_unsent_folder: Option<String>`, `DesktopConfig.kept_unsent_unfinished: Vec<String>`.
  - `config::ConfigFs` (trait: `write`, `sync_file`, `rename`, `sync_dir`), `config::RealConfigFs`, `DesktopConfig::update_at_durable(path, change)`, `DesktopConfig::update_at_durable_with(path, change, &dyn ConfigFs)`.
  - `kept_record::{record_kept_unsent, take_back_kept_unsent, dismiss_kept_unsent, save_kept_unsent_durably, take_back_kept_unsent_at, clear_unfinished_at, dismiss_saved_kept_unsent_at, saved_kept_unsent_at, unfinished_at}`:
    - `save_kept_unsent_durably(path: &Path, folder: &str, fs: &dyn ConfigFs) -> Result<Option<String>, String>` returns the `kept_unsent_folder` it replaced;
    - `take_back_kept_unsent_at(path: &Path, folder: &str, previous: Option<&str>) -> Result<(), String>`;
    - `clear_unfinished_at(path: &Path) -> Result<(), String>`;
    - `dismiss_saved_kept_unsent_at(path: &Path, shown: &str) -> Result<finder_removal::DismissOutcome, String>`;
    - `saved_kept_unsent_at(path) -> Result<Option<String>, String>`, `unfinished_at(path) -> Result<Vec<String>, String>`.

- [ ] **Step 1: Write the failing tests** in `kept_record.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigFs, DesktopConfig, DesktopSettings, RealConfigFs};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    fn config_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.toml");
        (dir, path)
    }

    /// U22.
    #[test]
    fn kept_unsent_folder_follows_1882s_rules_and_its_own_key() {
        let (_dir, path) = config_file();
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/kept").unwrap();
        save_kept_unsent_durably(&path, "/ours/one", &RealConfigFs).unwrap();
        assert_eq!(save_kept_unsent_durably(&path, "/ours/two", &RealConfigFs).unwrap().as_deref(), Some("/ours/one"));
        assert_eq!(saved_kept_unsent_at(&path).unwrap().as_deref(), Some("/ours/two"), "a newer folder replaces an older one");
        // A settings save, as `set_desktop_config` makes it, cannot clear it.
        let settings = DesktopSettings::from(&DesktopConfig::load_from(&path).unwrap());
        DesktopConfig::update_at(&path, |cfg| {
            cfg.apply_settings(settings);
            (true, ())
        })
        .unwrap();
        assert_eq!(saved_kept_unsent_at(&path).unwrap().as_deref(), Some("/ours/two"));
        // Dismiss clears only the folder the row showed.
        let stale = dismiss_saved_kept_unsent_at(&path, "/ours/one").unwrap();
        assert_eq!((stale.cleared, stale.current.as_deref()), (false, Some("/ours/two")));
        let shown = dismiss_saved_kept_unsent_at(&path, "/ours/two").unwrap();
        assert_eq!((shown.cleared, shown.current), (true, None));
        // The two keys never touch each other.
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/newer").unwrap();
        assert_eq!(unfinished_at(&path).unwrap(), ["/ours/one", "/ours/two"], "1882's save leaves our list alone");
    }

    #[derive(Default)]
    struct Recorded {
        steps: RefCell<Vec<String>>,
        fail_at: Option<&'static str>,
    }

    impl Recorded {
        fn step(&self, name: &str, path: &Path) -> std::io::Result<()> {
            self.steps.borrow_mut().push(format!("{name} {}", path.file_name().unwrap().to_string_lossy()));
            if self.fail_at == Some(name) {
                return Err(std::io::Error::other("injected"));
            }
            Ok(())
        }
    }

    impl ConfigFs for Recorded {
        fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            self.step("write", path)?;
            RealConfigFs.write(path, bytes)
        }
        fn sync_file(&self, path: &Path) -> std::io::Result<()> {
            self.step("sync_file", path)
        }
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.step("rename", to)?;
            RealConfigFs.rename(from, to)
        }
        fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
            self.step("sync_dir", dir)
        }
    }

    /// U43, the save's half (Task 5 has the keep step's take-back and folder removal).
    #[test]
    fn the_marker_save_is_one_durable_write() {
        let (dir, path) = config_file();
        crate::finder_removal::remember_kept_folder_at(&path, "/macos/kept").unwrap();
        let fs = Recorded::default();
        save_kept_unsent_durably(&path, "/ours/one", &fs).unwrap();
        let dir_name = dir.path().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            *fs.steps.borrow(),
            [
                "write desktop.toml.tmp".to_string(),
                "sync_file desktop.toml.tmp".to_string(),
                "rename desktop.toml".to_string(),
                format!("sync_dir {dir_name}"),
            ],
            "one write, one flush, ONE rename for both keys, then the directory"
        );
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(cfg.kept_unsent_folder.as_deref(), Some("/ours/one"));
        assert_eq!(cfg.kept_unsent_unfinished, ["/ours/one"]);
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));

        let before_rename = Recorded { fail_at: Some("sync_file"), ..Default::default() };
        assert!(save_kept_unsent_durably(&path, "/ours/two", &before_rename).is_err());
        assert_eq!(unfinished_at(&path).unwrap(), ["/ours/one"], "a failure before the rename changes nothing");

        let after_rename = Recorded { fail_at: Some("sync_dir"), ..Default::default() };
        let previous = save_kept_unsent_durably(&path, "/ours/three", &after_rename);
        assert!(previous.is_err());
        take_back_kept_unsent_at(&path, "/ours/three", Some("/ours/one")).unwrap();
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert_eq!(cfg.kept_unsent_unfinished, ["/ours/one"], "the take-back leaves no key naming the folder");
        assert_eq!(cfg.kept_unsent_folder.as_deref(), Some("/ours/one"), "and puts the older row back");

        // Every other save stays as it was: no flush in `save_to` (spec §7.3, amended for 1889).
        let config_rs = include_str!("../config.rs");
        let from = config_rs.find("    fn save_to(&self, path: &Path)").expect("save_to");
        let save_to = &config_rs[from..from + config_rs[from..].find("\n    }\n").unwrap()];
        assert!(!save_to.contains("sync_all"), "{save_to}");
    }

    #[test]
    fn a_completed_purge_clears_the_unfinished_list_and_nothing_else() {
        let (_dir, path) = config_file();
        save_kept_unsent_durably(&path, "/ours/one", &RealConfigFs).unwrap();
        save_kept_unsent_durably(&path, "/ours/two", &RealConfigFs).unwrap();
        clear_unfinished_at(&path).unwrap();
        let cfg = DesktopConfig::load_from(&path).unwrap();
        assert!(cfg.kept_unsent_unfinished.is_empty());
        assert_eq!(cfg.kept_unsent_folder.as_deref(), Some("/ours/two"), "the row stays");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("kept_unsent_unfinished"), "absent from the file while empty: {text}");
    }
}
```

- [ ] **Step 2: Run them, expect FAIL.** Filter `sign_out::kept_record`. Expected: compile errors.

- [ ] **Step 3: Implement.** In `config.rs`, after `kept_unsynced_folder`:

```rust
    /// Spec 2026-10-10 W9 (task 1887): the folder a sign-out last copied changes that had not reached the server into.
    /// Settings › Sync shows it until it is dismissed. Saved by the keep step before the purge; never in
    /// `DesktopSettings`, so a settings save cannot clear it; never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_unsent_folder: Option<String>,

    /// Spec 2026-10-10 W13 (task 1887): every folder whose sign-out was interrupted after its keep step, oldest first.
    /// Saved with `kept_unsent_folder` in one durable write before the purge's first deletion; a later keep adds to
    /// it; only a purge that cleared the rows clears it. Absent from the file while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_unsent_unfinished: Vec<String>,
```

with `kept_unsent_folder: None, kept_unsent_unfinished: Vec::new(),` in the `Default` impl after `kept_unsynced_folder: None,`. Next to `try_update_at_then`:

```rust
/// The file-system steps of a durable save, so a test can record their order and fail one (spec 2026-10-10 U43).
pub(crate) trait ConfigFs {
    /// Write `bytes` to `path`, owner-only.
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_file(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
}

pub(crate) struct RealConfigFs;

impl ConfigFs for RealConfigFs {
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        fs::File::open(path)?.sync_all()
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        fs::File::open(dir)?.sync_all()
    }
}
```

and in `impl DesktopConfig`:

```rust
    /// Spec 2026-10-10 §7.3 (task 1887): `update_at`, whose write survives a power loss once it returns: the temp file
    /// is flushed before the rename, and the directory after it. Only the keep step's save uses it; every other save
    /// stays as it is.
    pub(crate) fn update_at_durable<R>(path: &Path, change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, String> {
        Self::update_at_durable_with(path, change, &RealConfigFs)
    }

    /// [`Self::update_at_durable`] with its file-system steps passed in.
    pub(crate) fn update_at_durable_with<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        fs_steps: &dyn ConfigFs,
    ) -> Result<R, String> {
        let _writing = config_write_guard();
        let mut cfg = Self::load_from(path)?;
        let (save, value) = change(&mut cfg);
        if save {
            cfg.save_to_durable(path, fs_steps)?;
        }
        Ok(value)
    }

    /// The durable write. The caller holds the config-write lock.
    fn save_to_durable(&self, path: &Path, fs_steps: &dyn ConfigFs) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        fs_steps.write(&tmp, toml_str.as_bytes()).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs_steps.sync_file(&tmp).map_err(|e| format!("flush {}: {e}", tmp.display()))?;
        fs_steps.rename(&tmp, path).map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;
        let dir = path.parent().ok_or_else(|| "the config file has no directory".to_string())?;
        fs_steps.sync_dir(dir).map_err(|e| format!("flush {}: {e}", dir.display()))
    }
```

Then `src-tauri/src/sign_out/kept_record.rs` above its tests:

```rust
//! Spec 2026-10-10 W9, W13 and §9 (task 1887): the kept folder's two keys in `desktop.toml`. Paths are never logged.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::path::Path;

use crate::config::{ConfigFs, DesktopConfig};
use crate::finder_removal::DismissOutcome;

/// The keep step's record: the row's folder, and the folder added to the unfinished list (unless it is there).
/// Returns the `kept_unsent_folder` it replaced.
pub(crate) fn record_kept_unsent(cfg: &mut DesktopConfig, folder: &str) -> Option<String> {
    if !cfg.kept_unsent_unfinished.iter().any(|known| known == folder) {
        cfg.kept_unsent_unfinished.push(folder.to_string());
    }
    cfg.kept_unsent_folder.replace(folder.to_string())
}

/// A failed keep step after its save (W8): no key may name the folder it is about to remove.
pub(crate) fn take_back_kept_unsent(cfg: &mut DesktopConfig, folder: &str, previous: Option<&str>) -> bool {
    let before = (cfg.kept_unsent_folder.clone(), cfg.kept_unsent_unfinished.len());
    cfg.kept_unsent_unfinished.retain(|known| known != folder);
    if cfg.kept_unsent_folder.as_deref() == Some(folder) {
        cfg.kept_unsent_folder = previous.map(str::to_string);
    }
    before != (cfg.kept_unsent_folder.clone(), cfg.kept_unsent_unfinished.len())
}

/// The row's "Dismiss": clears only the folder the row showed (1882's rule, `finder_removal::dismiss_kept_folder`).
pub(crate) fn dismiss_kept_unsent(cfg: &mut DesktopConfig, shown: &str) -> bool {
    if cfg.kept_unsent_folder.as_deref() != Some(shown) {
        return false;
    }
    cfg.kept_unsent_folder = None;
    true
}

/// W9 and W13: both keys in ONE durable save, the keep step's last act.
pub(crate) fn save_kept_unsent_durably(path: &Path, folder: &str, fs_steps: &dyn ConfigFs) -> Result<Option<String>, String> {
    DesktopConfig::update_at_durable_with(path, |cfg| (true, record_kept_unsent(cfg, folder)), fs_steps)
}

pub(crate) fn take_back_kept_unsent_at(path: &Path, folder: &str, previous: Option<&str>) -> Result<(), String> {
    DesktopConfig::update_at(path, |cfg| (take_back_kept_unsent(cfg, folder, previous), ()))
}

/// W13: a purge that cleared the rows clears the list (plain, not durable: a lost clear only repeats A3e once).
pub(crate) fn clear_unfinished_at(path: &Path) -> Result<(), String> {
    DesktopConfig::update_at(path, |cfg| {
        let changed = !cfg.kept_unsent_unfinished.is_empty();
        cfg.kept_unsent_unfinished.clear();
        (changed, ())
    })
}

pub(crate) fn dismiss_saved_kept_unsent_at(path: &Path, shown: &str) -> Result<DismissOutcome, String> {
    DesktopConfig::update_at(path, |cfg| {
        let cleared = dismiss_kept_unsent(cfg, shown);
        (cleared, DismissOutcome { cleared, current: cfg.kept_unsent_folder.clone() })
    })
}

pub(crate) fn saved_kept_unsent_at(path: &Path) -> Result<Option<String>, String> {
    Ok(DesktopConfig::load_from(path)?.kept_unsent_folder)
}

pub(crate) fn unfinished_at(path: &Path) -> Result<Vec<String>, String> {
    Ok(DesktopConfig::load_from(path)?.kept_unsent_unfinished)
}
```

Add `pub(crate) mod kept_record;` to `sign_out/mod.rs`.

- [ ] **Step 4: Run, expect PASS.** Filter `sign_out::kept_record`: `ok. 3 passed`. Then the 1889 pins, filter `test_1889`: the same count as at Task 1's baseline run (`grep -c "test_1889" $EVID/t0-cargo-test.log` for the number).

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Store our folder in `kept_unsynced_folder`: U22 red on the 1882 key.
  2. `record_kept_unsent` replaces the list (`cfg.kept_unsent_unfinished = vec![folder.into()]`): U22 red on `["/ours/one", "/ours/two"]`.
  3. Drop `sync_file` in `save_to_durable`: U43 red on the step list.
  4. Save the two keys in two `update_at_durable_with` calls: U43 red (two renames).
  5. `take_back_kept_unsent` leaves `kept_unsent_folder` as it is: U43 red on "puts the older row back".
  6. Add a `sync_all` to `save_to`: U43 red on the last assertion.

- [ ] **Step 6: The task gate** with `<N>` = 4 and `<L>` = `L0 + 22`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/config.rs src-tauri/src/sign_out/kept_record.rs src-tauri/src/sign_out/mod.rs
git commit -m "feat(macos): record a kept folder and its unfinished marker in one durable save (1887)" -- src-tauri/src/config.rs src-tauri/src/sign_out/kept_record.rs src-tauri/src/sign_out/mod.rs
```

### Task 5: The keep step

**Files:**
- Create: `src-tauri/src/sign_out/kept_set.rs`, `src-tauri/src/sign_out/keep.rs` (macOS only), `src-tauri/src/sign_out/copy.rs` (the three error sentences now; Task 10 adds the rest)
- Modify: `src-tauri/src/sign_out/mod.rs`
- Test: `src-tauri/src/sign_out/keep.rs`

**Interfaces:**
- Consumes: `WaitingChanges` (Task 1); `kept_names::{kept_relative_path, KeptRole}` (Task 2); `KeptWriter`, `CloneFile`, `SystemClone` (Task 3); `kept_record::{save_kept_unsent_durably, take_back_kept_unsent_at, saved_kept_unsent_at}`, `ConfigFs`, `RealConfigFs` (Task 4).
- Produces:
  - `kept_set::KeptSet { copied, covered, missing_acknowledged, others_acknowledged: BTreeSet<String>, folder: Option<PathBuf>, files: usize }`, `KeptSet::keeps(&self, op_id: &str) -> bool`.
  - `kept_set::Acknowledge<'a>`: `Asked { others: &'a BTreeSet<String>, missing: &'a BTreeSet<String> }` (the snapshot the gate accepted), `Nothing` (no snapshot: the gate counted nothing), `Everything` (the R10 reset, W12).
  - `keep::{KeepInputs, KeepFailure, keep_unsent}`: `keep_unsent(KeepInputs) -> Result<KeptSet, KeepFailure>`; `KeepFailure::{Unsent, Copy(&'static str), Read(String)}` with `.message() -> String` (code-prefixed, §11).
  - `copy::{UNSENT_CHANGES_CODE, UNSENT_COPY_FAILED_CODE, UNSENT_COUNT_FAILED_CODE, E1_UNSENT_CHANGES, E2_UNSENT_COPY_FAILED, E3_UNSENT_COUNT_FAILED, coded}`.

- [ ] **Step 1: Write the failing tests** in `keep.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RealConfigFs;
    use crate::sign_out::kept_writer::SystemClone;
    use crate::state_db::StateDb;
    use sha2::{Digest, Sha256};
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    struct Fixture {
        dir: tempfile::TempDir,
        db: StateDb,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            Self { dir, db }
        }
        fn root(&self) -> PathBuf {
            self.dir.path().join("Documents").join("Changes not sent")
        }
        fn config(&self) -> PathBuf {
            self.dir.path().join("desktop.toml")
        }
        fn file(&self, file_id: &str, path: &str) {
            self.db
                .hold_lock_for_test()
                .execute("INSERT INTO files (file_id, path) VALUES (?1, ?2)", rusqlite::params![file_id, path])
                .unwrap();
        }
        /// A waiting upload whose staged copy holds `bytes` (`None`: the copy is gone).
        fn write(&self, op_id: &str, file_id: &str, origin: &str, bytes: Option<&[u8]>) -> PathBuf {
            let staged = self.dir.path().join("staging").join(op_id);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            if let Some(bytes) = bytes {
                std::fs::write(&staged, bytes).unwrap();
            }
            self.db
                .hold_lock_for_test()
                .execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                     VALUES (?1, 'upload_version', ?2, ?3, ?1, ?4)",
                    rusqlite::params![op_id, file_id, staged.to_str().unwrap(), origin],
                )
                .unwrap();
            staged
        }
        fn other(&self, op_id: &str, kind: &str, file_id: &str) {
            self.db
                .hold_lock_for_test()
                .execute(
                    "INSERT INTO operation_queue (op_id, kind, file_id) VALUES (?1, ?2, ?3)",
                    rusqlite::params![op_id, kind, file_id],
                )
                .unwrap();
        }
        fn keep_with(&self, acknowledge: Acknowledge<'_>, clone: &dyn CloneFile, fs: &dyn ConfigFs) -> Result<KeptSet, KeepFailure> {
            let root = self.root();
            let config = self.config();
            keep_unsent(KeepInputs {
                db: &self.db,
                acknowledge,
                root: &root,
                event_name: "2026-10-10 at 14.05.12".into(),
                config_path: &config,
                clone,
                config_fs: fs,
            })
        }
        fn keep(&self, acknowledge: Acknowledge<'_>) -> Result<KeptSet, KeepFailure> {
            self.keep_with(acknowledge, &SystemClone, &RealConfigFs)
        }
        fn event(&self) -> PathBuf {
            self.root().join("2026-10-10 at 14.05.12")
        }
        fn queued(&self) -> usize {
            self.db.waiting_rows().unwrap().len()
        }
    }

    fn sha(bytes: &[u8]) -> Vec<u8> {
        Sha256::digest(bytes).to_vec()
    }

    fn sha_of(path: &Path) -> Vec<u8> {
        sha(&std::fs::read(path).unwrap())
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    struct FailingClone {
        calls: Cell<usize>,
        fail_on: usize,
    }

    impl CloneFile for FailingClone {
        fn clone_file(&self, src: &Path, dst: &Path) -> std::io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == self.fail_on {
                return Err(std::io::Error::other("injected"));
            }
            SystemClone.clone_file(src, dst)
        }
    }

    /// U8.
    #[test]
    fn sign_out_keeps_the_kept_set_before_the_purge() {
        let f = Fixture::new();
        f.file("report", "Work/report.docx");
        f.file("notes", "New/notes.txt");
        f.file("a", "Old/a.txt");
        f.write("s1", "report", "minted", Some(b"first save"));
        f.write("s2", "report", "minted", Some(b"second save"));
        f.write("c1", "notes", "minted", Some(b"notes"));
        f.write("a-old", "a", "earlier_build", Some(b"earlier build"));
        f.write("a-new", "a", "minted", Some(b"later save"));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        let event = f.event();
        assert_eq!(sha_of(&event.join("Work/report.docx")), sha(b"second save"));
        assert_eq!(sha_of(&event.join("New/notes.txt")), sha(b"notes"));
        assert_eq!(sha_of(&event.join("Old/a.txt")), sha(b"later save"));
        assert_eq!(sha_of(&event.join("Old/a (earlier save 1).txt")), sha(b"earlier build"));
        assert!(!event.join("Work/report (earlier save 1).docx").exists(), "a minted earlier save is covered, never copied");
        assert_eq!(kept.copied, set(&["s2", "c1", "a-old", "a-new"]));
        assert_eq!(kept.covered, set(&["s1"]));
        assert_eq!((kept.folder.as_deref(), kept.files), (Some(event.as_path()), 4));
    }

    /// U8b.
    #[test]
    fn two_minted_saves_of_one_file_keep_one_file() {
        let f = Fixture::new();
        f.file("report", "Work/report.docx");
        f.write("s1", "report", "minted", Some(b"older"));
        f.write("s2", "report", "minted", Some(b"newer"));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(std::fs::read_dir(f.event().join("Work")).unwrap().count(), 1);
        assert_eq!(sha_of(&f.event().join("Work/report.docx")), sha(b"newer"));
        assert!(kept.keeps("s1") && kept.covered.contains("s1"));
    }

    /// U13, the keep half (Task 8 has the sign-out half: keys in memory, sync restarts).
    #[test]
    fn a_failed_copy_stops_the_sign_out() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("b", "b.txt");
        let a = f.write("wa", "a", "minted", Some(b"a"));
        let b = f.write("wb", "b", "minted", Some(b"b"));
        let failing = FailingClone { calls: Cell::new(0), fail_on: 2 };
        let failure = f.keep_with(Acknowledge::Nothing, &failing, &RealConfigFs).unwrap_err();
        assert!(failure.message().starts_with("unsent_copy_failed: "), "{}", failure.message());
        assert!(!f.event().exists(), "the half-made folder is removed");
        assert_eq!(crate::sign_out::kept_record::saved_kept_unsent_at(&f.config()).unwrap(), None, "no record");
        assert_eq!(f.queued(), 2, "the queue is intact");
        assert!(a.exists() && b.exists(), "the staged copies are intact");
    }

    /// U27, the keep half (Task 6 has the purge half).
    #[test]
    fn the_keep_step_copies_from_either_staging_root() {
        let f = Fixture::new();
        f.file("x", "x.txt");
        f.file("y", "y.txt");
        let caches = f.dir.path().join("Library/Caches/beebeeb/finder-writes/old-copy");
        let support = f.dir.path().join("Library/Application Support/beebeeb/finder-writes/new-copy");
        for (path, bytes) in [(&caches, b"from caches"), (&support, b"from support")] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        f.db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
                   ('ox', 'upload_version', 'x', '{}', 'ox', 'minted'),
                   ('oy', 'upload_version', 'y', '{}', 'oy', 'minted');",
                caches.display(),
                support.display()
            ))
            .unwrap();
        f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(sha_of(&f.event().join("x.txt")), sha(b"from caches"));
        assert_eq!(sha_of(&f.event().join("y.txt")), sha(b"from support"));
    }

    /// U29 cases a and c.
    #[test]
    fn a_copy_missing_since_the_ask_stops_the_sign_out() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("b", "b.txt");
        let a = f.write("wa", "a", "minted", Some(b"a"));
        f.write("wb", "b", "minted", Some(b"b"));
        std::fs::remove_file(&a).unwrap(); // gone after an ask that showed no missing write
        let (others, missing) = (set(&[]), set(&[]));
        let failure = f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap_err();
        assert!(failure.message().starts_with("unsent_copy_failed: "));
        assert!(!f.event().exists(), "nothing copied");
        assert_eq!(f.queued(), 2, "nothing purged");
        // Case c: the ask showed it missing.
        let missing = set(&["wa"]);
        let kept = f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap();
        assert!(kept.missing_acknowledged.contains("wa"));
        assert_eq!(sha_of(&f.event().join("b.txt")), sha(b"b"), "the other writes are kept");
    }

    /// U36, the keep half.
    #[test]
    fn a_late_other_change_stops_the_sign_out_before_any_copy() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        f.other("r1", "rename_file", "a");
        f.other("r2", "rename_file", "a");
        let (others, missing) = (set(&["r1"]), set(&[]));
        let failure = f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap_err();
        assert!(matches!(failure, KeepFailure::Unsent));
        assert!(failure.message().starts_with("unsent_changes: "));
        assert!(!f.root().exists(), "refused before any directory was made");
        // A write that arrived after the gate, with its copy, is kept and counted (U9's case, with a snapshot).
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.file("late", "late.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        f.write("wl", "late", "minted", Some(b"late"));
        let (others, missing) = (set(&[]), set(&[]));
        let kept = f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap();
        assert_eq!(kept.files, 2);
    }

    /// U38, the keep half; U15, the copy half.
    #[test]
    fn only_files_with_a_copy_are_copied_and_completed_uploads_never() {
        let f = Fixture::new();
        f.file("x", "x.txt");
        f.file("y", "y.txt");
        f.write("x-old", "x", "earlier_build", None);
        f.write("x-new", "x", "minted", Some(b"x"));
        f.write("y", "y", "minted", None);
        f.write("done", "z", "minted", Some(b"done"));
        f.db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO upload_resume (op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                   server_file_id, object_version_id, chunk_size_bytes, chunk_count, completed_version)
                 VALUES ('done', 'p', 1, 1, 'u', 'z', 'v', 1, 1, 3);",
            )
            .unwrap();
        let (others, missing) = (set(&[]), set(&["x-old", "y"]));
        let kept = f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap();
        assert_eq!(kept.files, 1, "X is copied, Y is not, the completed upload never");
        assert!(!kept.keeps("done"));
    }

    /// Review Focus 3.
    #[test]
    fn an_empty_save_is_kept_as_an_empty_file() {
        let f = Fixture::new();
        f.file("e", "empty.txt");
        f.write("we", "e", "minted", Some(b""));
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert!(kept.copied.contains("we"));
        assert_eq!(std::fs::metadata(f.event().join("empty.txt")).unwrap().len(), 0);
    }

    /// Review Focus 4: the name is the row's path at the keep step, never an op's `target_path`.
    #[test]
    fn a_waiting_rename_names_the_kept_copy_as_finder_shows_it() {
        let f = Fixture::new();
        f.file("r", "Work/new name.docx");
        f.write("wr", "r", "minted", Some(b"content"));
        f.db.hold_lock_for_test()
            .execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, target_path) VALUES ('rn', 'rename_file', 'r', 'old name.docx')",
                [],
            )
            .unwrap();
        let (others, missing) = (set(&["rn"]), set(&[]));
        f.keep(Acknowledge::Asked { others: &others, missing: &missing }).unwrap();
        assert_eq!(sha_of(&f.event().join("Work/new name.docx")), sha(b"content"));
    }

    /// Review Focus 5.
    #[test]
    fn five_hundred_waiting_files_are_counted_and_kept_once_each() {
        let f = Fixture::new();
        for i in 0..500 {
            f.file(&format!("f{i}"), &format!("Drop/file {i:03}.bin"));
            f.write(&format!("w{i}"), &format!("f{i}"), "minted", Some(format!("bytes {i}").as_bytes()));
        }
        assert_eq!(crate::sign_out::waiting::WaitingChanges::read(&f.db).unwrap().counts().files, 500);
        let kept = f.keep(Acknowledge::Nothing).unwrap();
        assert_eq!(kept.files, 500);
        for i in [0, 250, 499] {
            assert_eq!(sha_of(&f.event().join(format!("Drop/file {i:03}.bin"))), sha(format!("bytes {i}").as_bytes()));
        }
    }

    /// U43, the keep step's half: a save that fails after its rename names nothing, and the folder goes.
    #[test]
    fn a_failed_marker_save_removes_the_folder_and_names_nothing() {
        struct FailDirFlush;
        impl ConfigFs for FailDirFlush {
            fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> { RealConfigFs.write(path, bytes) }
            fn sync_file(&self, path: &Path) -> std::io::Result<()> { RealConfigFs.sync_file(path) }
            fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> { RealConfigFs.rename(from, to) }
            fn sync_dir(&self, _dir: &Path) -> std::io::Result<()> { Err(std::io::Error::other("injected")) }
        }
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("wa", "a", "minted", Some(b"a"));
        let failure = f.keep_with(Acknowledge::Nothing, &SystemClone, &FailDirFlush).unwrap_err();
        assert!(failure.message().starts_with("unsent_copy_failed: "));
        assert!(!f.event().exists());
        let cfg = crate::config::DesktopConfig::load_from(&f.config()).unwrap();
        assert_eq!((cfg.kept_unsent_folder, cfg.kept_unsent_unfinished), (None, Vec::<String>::new()));
    }

    /// W12: the R10 reset acknowledges everything it cannot ask about.
    #[test]
    fn the_reset_acknowledges_missing_writes_and_other_changes() {
        let f = Fixture::new();
        f.file("a", "a.txt");
        f.write("gone", "a", "minted", None);
        f.other("r1", "rename_file", "a");
        let kept = f.keep(Acknowledge::Everything).unwrap();
        assert!(kept.missing_acknowledged.contains("gone") && kept.others_acknowledged.contains("r1"));
        assert_eq!(kept.folder, None, "no copy, no folder");
    }
}
```

- [ ] **Step 2: Run them, expect FAIL.** Filter `sign_out::keep::`. Expected: compile errors.

- [ ] **Step 3: Implement.** `src-tauri/src/sign_out/copy.rs`:

```rust
//! Spec 2026-10-10 §11 (task 1887): the strings Rust shows or returns. Typographic apostrophes, the one-character
//! ellipsis, no emoji. `src/signOutCopy.ts` mirrors them; `tests/signOutCopy.test.ts` pins the two equal.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

pub const UNSENT_CHANGES_CODE: &str = "unsent_changes";
pub const UNSENT_COPY_FAILED_CODE: &str = "unsent_copy_failed";
pub const UNSENT_COUNT_FAILED_CODE: &str = "unsent_count_failed";

pub const E1_UNSENT_CHANGES: &str =
    "Some changes haven’t reached the server yet. Sign out from Settings › Account to send or keep them first.";
pub const E2_UNSENT_COPY_FAILED: &str = "Beebeeb couldn’t copy the changes that haven’t reached the server, so it didn’t sign out. They’re still waiting to upload. Try again; if it keeps happening, restart Beebeeb.";
pub const E3_UNSENT_COUNT_FAILED: &str = "Beebeeb couldn’t check for changes that haven’t reached the server, so it changed nothing. Try again. If it keeps happening, restart Beebeeb.";

/// §11: `<code>: <sentence>`. A caller matches the code, never the sentence.
pub fn coded(code: &str, sentence: &str) -> String {
    format!("{code}: {sentence}")
}
```

`src-tauri/src/sign_out/kept_set.rs`:

```rust
//! Spec 2026-10-10 W6 (task 1887): what the keep step kept, by op id, for the purge guard.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeptSet {
    pub copied: BTreeSet<String>,
    pub covered: BTreeSet<String>,
    pub missing_acknowledged: BTreeSet<String>,
    pub others_acknowledged: BTreeSet<String>,
    /// The event folder, when a file was copied.
    pub folder: Option<PathBuf>,
    pub files: usize,
}

impl KeptSet {
    /// Copied, covered, or a missing write the person was shown: the purge may delete it.
    pub fn keeps(&self, op_id: &str) -> bool {
        self.copied.contains(op_id) || self.covered.contains(op_id) || self.missing_acknowledged.contains(op_id)
    }
}

/// What the keep step may let go without a copy (W6, W7, W12).
#[derive(Debug, Clone, Copy)]
pub enum Acknowledge<'a> {
    /// The ask snapshot the gate accepted: its other changes and its missing writes.
    Asked { others: &'a BTreeSet<String>, missing: &'a BTreeSet<String> },
    /// No snapshot (the gate counted nothing): nothing but copies.
    Nothing,
    /// The R10 reset asks nobody (W12).
    Everything,
}

impl Acknowledge<'_> {
    pub fn other(&self, op_id: &str) -> bool {
        match self {
            Acknowledge::Asked { others, .. } => others.contains(op_id),
            Acknowledge::Nothing => false,
            Acknowledge::Everything => true,
        }
    }

    pub fn missing(&self, op_id: &str) -> bool {
        match self {
            Acknowledge::Asked { missing, .. } => missing.contains(op_id),
            Acknowledge::Nothing => false,
            Acknowledge::Everything => true,
        }
    }
}
```

`src-tauri/src/sign_out/keep.rs` above its tests:

```rust
//! Spec 2026-10-10 W6-W9 and §5.2 step 7 (task 1887, macOS): after the confirmed engine stop and before the purge,
//! copy every waiting save that can be kept into a new kept folder, flush it, and record it.

use std::path::Path;

use crate::config::ConfigFs;
use crate::state_db::StateDb;

use super::copy::{self, E1_UNSENT_CHANGES, E2_UNSENT_COPY_FAILED, E3_UNSENT_COUNT_FAILED};
use super::kept_names::{KeptRole, kept_relative_path};
pub(crate) use super::kept_set::{Acknowledge, KeptSet};
use super::kept_record::{save_kept_unsent_durably, saved_kept_unsent_at, take_back_kept_unsent_at};
pub(crate) use super::kept_writer::CloneFile;
use super::kept_writer::KeptWriter;
use super::waiting::{WaitingChanges, WriteState};

pub(crate) struct KeepInputs<'a> {
    pub db: &'a StateDb,
    pub acknowledge: Acknowledge<'a>,
    pub root: &'a Path,
    pub event_name: String,
    pub config_path: &'a Path,
    pub clone: &'a dyn CloneFile,
    pub config_fs: &'a dyn ConfigFs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeepFailure {
    /// An other change the person was not asked about (W6): the ask is shown again.
    Unsent,
    /// E2, with a category for the log (never a path).
    Copy(&'static str),
    /// The queue could not be read (E3).
    Read(String),
}

impl KeepFailure {
    pub(crate) fn message(&self) -> String {
        match self {
            KeepFailure::Unsent => copy::coded(copy::UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES),
            KeepFailure::Copy(_) => copy::coded(copy::UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
            KeepFailure::Read(_) => copy::coded(copy::UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
        }
    }
}

fn category(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::NotFound => "source_gone",
        std::io::ErrorKind::PermissionDenied => "permission",
        std::io::ErrorKind::StorageFull => "disk_full",
        _ => "io",
    }
}

/// W6: one read; refuse a late other change before any copy; refuse a missing write nobody was shown (W7); copy the
/// copied set; flush; record (W9, W13) as the last act. A failure before the record removes the half-made folder
/// (W8). Never deletes a queue row or a staged copy.
pub(crate) fn keep_unsent(inputs: KeepInputs<'_>) -> Result<KeptSet, KeepFailure> {
    let waiting = WaitingChanges::read(inputs.db).map_err(KeepFailure::Read)?;
    let mut kept = KeptSet::default();
    for other in &waiting.others {
        if !inputs.acknowledge.other(&other.op_id) {
            return Err(KeepFailure::Unsent);
        }
        kept.others_acknowledged.insert(other.op_id.clone());
    }
    for write in waiting.files.iter().flat_map(|file| &file.writes) {
        match write.state {
            WriteState::Missing if !inputs.acknowledge.missing(&write.op_id) => {
                return Err(KeepFailure::Copy("missing_not_shown"));
            }
            WriteState::Missing => {
                kept.missing_acknowledged.insert(write.op_id.clone());
            }
            WriteState::Covered => {
                kept.covered.insert(write.op_id.clone());
            }
            WriteState::Copied => {}
        }
    }
    let to_copy: Vec<_> = waiting.files.iter().filter(|file| file.has_copy()).collect();
    if to_copy.is_empty() {
        log_kept(&kept);
        return Ok(kept);
    }
    let mut writer = KeptWriter::create(inputs.root, &inputs.event_name, inputs.clone)
        .map_err(|error| KeepFailure::Copy(category(&error)))?;
    for file in &to_copy {
        // Newest first: the newest copied write gets the plain name, then "(earlier save 1)", "(earlier save 2)"…
        let copied: Vec<_> = file.writes.iter().rev().filter(|write| write.state == WriteState::Copied).collect();
        for (index, write) in copied.iter().enumerate() {
            let role = if index == 0 { KeptRole::Newest } else { KeptRole::EarlierSave(index as u32) };
            let components = kept_relative_path(file.path.as_deref(), &write.op_id, role);
            let source = write.payload_path.as_deref().expect("a copied write has a staged copy");
            if let Err(error) = writer.keep(&components, source) {
                writer.abandon();
                return Err(KeepFailure::Copy(category(&error)));
            }
            kept.copied.insert(write.op_id.clone());
        }
    }
    let event = writer.event().to_path_buf();
    let folder = match writer.finish() {
        Ok(folder) => folder,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&event);
            return Err(KeepFailure::Copy(category(&error)));
        }
    };
    let folder_text = folder.path.to_string_lossy().into_owned();
    let previous = saved_kept_unsent_at(inputs.config_path).ok().flatten();
    if save_kept_unsent_durably(inputs.config_path, &folder_text, inputs.config_fs).is_err() {
        // Best effort: no key may name a folder that is about to be removed (§7.3).
        let _ = take_back_kept_unsent_at(inputs.config_path, &folder_text, previous.as_deref());
        let _ = std::fs::remove_dir_all(&folder.path);
        return Err(KeepFailure::Copy("record"));
    }
    kept.folder = Some(folder.path);
    kept.files = folder.files;
    log_kept(&kept);
    Ok(kept)
}

/// §7.2: counts only, never a path or a name (W10).
fn log_kept(kept: &KeptSet) {
    tracing::info!(
        files = kept.files,
        kept = kept.copied.len(),
        covered = kept.covered.len(),
        missing = kept.missing_acknowledged.len(),
        "kept unsent changes"
    );
}
```

`sign_out/mod.rs` gains:

```rust
pub(crate) mod copy;
pub(crate) mod kept_set;
#[cfg(target_os = "macos")]
pub(crate) mod keep;
```

- [ ] **Step 4: Run, expect PASS.** Filter `sign_out::keep::`. Expected: `ok. 12 passed; 0 failed`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Skip the copy loop (`to_copy` empty): U8 red on the first `sha_of` (no file).
  2. Copy every write with a staged copy, covered ones too: U8 red on "a minted earlier save is covered, never copied"; U8b red on the count.
  3. Skip earlier-build writes: U8 red on `a (earlier save 1).txt`.
  4. Ignore the clone error (`let _ = writer.keep(…)`): U13 red on `unsent_copy_failed`.
  5. Drop `writer.abandon()`: U13 red on "the half-made folder is removed".
  6. Save the record before the copies: U13 red on "no record".
  7. Rebuild the source as `<data>/beebeeb/finder-writes/<file name>`: U27 red (the Caches copy is not found, E2).
  8. Decide missing at keep time only (treat every `Missing` as acknowledged): U29 case a red.
  9. Accept any other change (`Acknowledge::other` always `true`): U36 red.
  10. Name from the op's `target_path`: Review Focus 4 red.
  11. Drop the take-back: the U43 keep-half red on `(None, [])`.

- [ ] **Step 6: The task gate** with `<N>` = 5 and `<L>` = `L0 + 34`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/keep.rs src-tauri/src/sign_out/kept_set.rs src-tauri/src/sign_out/copy.rs src-tauri/src/sign_out/mod.rs
git commit -m "feat(macos): keep every waiting save in a recorded folder before a sign-out purges (1887)" -- src-tauri/src/sign_out/keep.rs src-tauri/src/sign_out/kept_set.rs src-tauri/src/sign_out/copy.rs src-tauri/src/sign_out/mod.rs
```

### Task 6: The purge guard and its order

**Files:**
- Modify: `src-tauri/src/state_db.rs` (`PurgeGuardRead`, `StateDb::unkept_waiting_changes`, after `waiting_rows`)
- Modify: `src-tauri/src/lib.rs`: `purge_local_data_for_sign_out` (`@184:lib.rs:3259-3293`), `purge_local_state_files` (`:4102-4107`), `purge_local_state_files_in` (`:4207-4213`), `purge_local_state_files_as` (`:4216-4265`), and every existing caller of those four
- Test: `src-tauri/src/lib.rs`, `mod sign_out_keep_tests` (Task 2)

**Interfaces:**
- Consumes: `KeptSet` (Task 5), `keep_unsent` (Task 5, in the tests), `kept_record::{clear_unfinished_at, unfinished_at, saved_kept_unsent_at}` (Task 4), `copy::{coded, UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED}` (Task 5).
- Produces:
  - `state_db::PurgeGuardRead { unkept: Vec<String>, waiting_payloads: Vec<String> }`, `StateDb::unkept_waiting_changes(&self, kept: &KeptSet) -> rusqlite::Result<PurgeGuardRead>`.
  - `lib.rs`: `purge_local_data_for_sign_out(state_dir, sync_root, owe_finder_removal, kept: Option<&KeptSet>)`; `purge_local_state_files(db, sync_root, kept)`; `purge_local_state_files_in(db, sync_root, staging_dirs, kept)`; `purge_local_state_files_as(db, sync_root, staging_dirs, uid, kept)`; `purge_local_state_files_with(db, sync_root, staging_dirs, uid, kept, remove: &dyn Fn(&Path) -> std::io::Result<()>)`.
  - A guard refusal returns exactly `coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED)`, never wrapped in `SIGN_OUT_PURGE_FAILED`.

- [ ] **Step 1: Write the failing tests** in `sign_out_keep_tests`:

```rust
    use crate::config::RealConfigFs;
    use crate::sign_out::keep::{Acknowledge, KeepInputs, keep_unsent};
    use crate::sign_out::kept_record::{saved_kept_unsent_at, unfinished_at};
    use crate::sign_out::kept_writer::SystemClone;

    /// A state dir with `state.db` where the sign-out's purge looks for it, and a config file of its own.
    struct PurgeFixture {
        dir: tempfile::TempDir,
        db: state_db::StateDb,
    }

    impl PurgeFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(dir.path())).unwrap();
            Self { dir, db }
        }
        fn config(&self) -> PathBuf {
            self.dir.path().join("desktop.toml")
        }
        fn staged(&self, name: &str) -> PathBuf {
            self.dir.path().join("staging").join(name)
        }
        fn write(&self, op_id: &str, file_id: &str, bytes: &[u8]) -> PathBuf {
            let staged = self.staged(op_id);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(&staged, bytes).unwrap();
            self.db
                .hold_lock_for_test()
                .execute_batch(&format!(
                    "INSERT OR IGNORE INTO files (file_id, path) VALUES ('{file_id}', '{file_id}.txt');
                     INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                     VALUES ('{op_id}', 'upload_version', '{file_id}', '{}', '{op_id}', 'minted');
                     INSERT INTO staged_payloads (path, completed) VALUES ('{}', 0);",
                    staged.display(),
                    staged.display()
                ))
                .unwrap();
            staged
        }
        fn keep(&self) -> KeptSet {
            let root = self.dir.path().join("Documents").join("Changes not sent");
            let config = self.config();
            keep_unsent(KeepInputs {
                db: &self.db,
                acknowledge: Acknowledge::Nothing,
                root: &root,
                event_name: "event".into(),
                config_path: &config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .unwrap()
        }
        fn queued(&self) -> usize {
            self.db.waiting_rows().unwrap().len()
        }
    }

    /// U14.
    #[test]
    fn the_purge_guard_runs_before_any_file_is_deleted() {
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let kept = f.keep();
        let late = f.write("late", "late", b"late"); // the test seam: a waiting write after the keep step
        let error = purge_local_state_files_as(&f.db, None, &[], None, Some(&kept)).err().expect("the guard refuses");
        assert!(error.starts_with("unsent_copy_failed: "), "{error}");
        assert!(late.exists() && a.exists(), "no staged copy was touched");
        let folder = kept.folder.clone().unwrap();
        assert!(folder.join("a.txt").exists(), "the kept folder stays");
        assert_eq!(saved_kept_unsent_at(&f.config()).unwrap(), Some(folder.to_string_lossy().into_owned()));
        assert_eq!(f.queued(), 2, "no queue row is gone");
    }

    /// U39 (a) and (b); (c) is the async test below.
    #[test]
    fn a_purge_failure_after_the_keep_step_spares_waiting_copies_and_is_named_next_time() {
        // (a) A removal fails on an orphan: every waiting copy is still there.
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let b = f.write("wb", "b", b"b");
        let orphan = f.staged("orphan");
        std::fs::write(&orphan, b"orphan").unwrap();
        f.db.hold_lock_for_test()
            .execute("INSERT INTO staged_payloads (path, completed) VALUES (?1, 1)", rusqlite::params![orphan.to_str().unwrap()])
            .unwrap();
        let kept = f.keep();
        let fail_on_orphan = |path: &Path| {
            if path == orphan { Err(std::io::Error::other("injected")) } else { std::fs::remove_file(path) }
        };
        assert!(purge_local_state_files_with(&f.db, None, &[], None, Some(&kept), &fail_on_orphan).is_err());
        assert!(a.exists() && b.exists(), "the waiting writes' copies go last");
        assert_eq!(f.queued(), 2);
        let folder = kept.folder.unwrap().to_string_lossy().into_owned();
        assert_eq!(unfinished_at(&f.config()).unwrap(), [folder.clone()], "saved by the keep step, before the purge");

        // (b) A removal fails on the second waiting copy: the first is gone, and the next count shows it missing.
        let f = PurgeFixture::new();
        let a = f.write("wa", "a", b"a");
        let b = f.write("wb", "b", b"b");
        let kept = f.keep();
        let fail_on_b = |path: &Path| if path == b { Err(std::io::Error::other("injected")) } else { std::fs::remove_file(path) };
        assert!(purge_local_state_files_with(&f.db, None, &[], None, Some(&kept), &fail_on_b).is_err());
        assert!(!a.exists() && b.exists());
        let counts = crate::sign_out::waiting::WaitingChanges::read(&f.db).unwrap().counts();
        assert_eq!(counts.missing_files, 1);
        assert_eq!(unfinished_at(&f.config()).unwrap().len(), 1);
    }

    /// U39 (c): a later sign-out that completes clears the list and never touches 1882's key.
    #[tokio::test]
    async fn a_completed_purge_clears_the_unfinished_list() {
        let f = PurgeFixture::new();
        f.write("wa", "a", b"a");
        let config_dir = f.dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(config_dir, async {
                let config = DesktopConfig::path().unwrap();
                crate::finder_removal::remember_kept_folder_at(&config, "/macos/kept").unwrap();
                let root = f.dir.path().join("Documents").join("Changes not sent");
                let kept = keep_unsent(KeepInputs {
                    db: &f.db,
                    acknowledge: Acknowledge::Nothing,
                    root: &root,
                    event_name: "event".into(),
                    config_path: &config,
                    clone: &SystemClone,
                    config_fs: &RealConfigFs,
                })
                .unwrap();
                assert_eq!(unfinished_at(&config).unwrap().len(), 1);
                purge_local_data_for_sign_out(Ok(f.dir.path().to_path_buf()), None, false, Some(&kept)).unwrap();
                let cfg = DesktopConfig::load_from(&config).unwrap();
                assert!(cfg.kept_unsent_unfinished.is_empty(), "the rows are cleared, so nothing can look missing");
                assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/macos/kept"));
                assert!(cfg.kept_unsent_folder.is_some(), "the row stays until dismissed");
            })
            .await;
    }

    /// U27, the purge half: the purge removes both staging roots' sources after keeping them.
    #[test]
    fn the_purge_removes_sources_from_every_staging_root() {
        let f = PurgeFixture::new();
        let caches = f.dir.path().join("Library/Caches/beebeeb/finder-writes/x");
        let support = f.dir.path().join("Library/Application Support/beebeeb/finder-writes/y");
        for path in [&caches, &support] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"bytes").unwrap();
        }
        f.db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin) VALUES
                   ('ox', 'upload_version', 'x', '{}', 'ox', 'minted'), ('oy', 'upload_version', 'y', '{}', 'oy', 'minted');",
                caches.display(),
                support.display()
            ))
            .unwrap();
        let kept = f.keep();
        purge_local_state_files_as(&f.db, None, &[], None, Some(&kept)).unwrap();
        assert!(!caches.exists() && !support.exists());
        assert_eq!(f.queued(), 0);
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filter `sign_out_keep_tests`. Expected: compile errors (the new parameters, `purge_local_state_files_with`, `unkept_waiting_changes`).

- [ ] **Step 3: Implement.** In `state_db.rs`, after `waiting_rows`:

```rust
/// Spec 2026-10-10 §5.2 step 8: what the purge guard read. `unkept`: waiting changes the keep step did not keep or
/// acknowledge (the purge refuses). `waiting_payloads`: every waiting write's staged copy, deleted last (W13).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PurgeGuardRead {
    pub unkept: Vec<String>,
    pub waiting_payloads: Vec<String>,
}
```

```rust
    /// The purge guard's ONE read (spec 2026-10-10 W6, §5.2 step 8): one transaction, before any file is touched.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn unkept_waiting_changes(&self, kept: &crate::sign_out::kept_set::KeptSet) -> Result<PurgeGuardRead> {
        let mut read = PurgeGuardRead::default();
        for row in self.waiting_rows()? {
            match row.kind {
                OperationKind::UploadFile | OperationKind::UploadVersion => {
                    if row.completed {
                        continue;
                    }
                    if let Some(path) = &row.payload_path {
                        read.waiting_payloads.push(path.clone());
                    }
                    if !kept.keeps(&row.op_id) {
                        read.unkept.push(row.op_id);
                    }
                }
                OperationKind::HydrateFile | OperationKind::PinTree => {}
                _ => {
                    if !kept.others_acknowledged.contains(&row.op_id) {
                        read.unkept.push(row.op_id);
                    }
                }
            }
        }
        Ok(read)
    }
```

In `lib.rs`:

1. `purge_local_data_for_sign_out` gains `kept: Option<&sign_out::kept_set::KeptSet>` as its last parameter; it passes it to `purge_local_state_files(&db, sync_root, kept)`; a guard refusal passes through unwrapped; and after the rows are cleared it clears the marker (macOS):

```rust
        Ok(Some(db)) => match purge_local_state_files(&db, sync_root, kept) {
            Ok(summary) => {
                tracing::info!(
                    queued_ops_purged = summary.queued_ops_purged,
                    files_removed = summary.files_removed,
                    files_skipped = summary.files_skipped,
                    windows_placeholders_dehydrated = summary.windows_placeholders_dehydrated,
                    "purged operation queue and decrypted cache on sign-out"
                );
                // Spec 2026-10-10 W13: the rows are cleared, so no write can look missing any more.
                #[cfg(target_os = "macos")]
                if let Err(error) = DesktopConfig::path().and_then(|path| sign_out::kept_record::clear_unfinished_at(&path)) {
                    tracing::warn!(error = %error, "could not clear the unfinished sign-out marker");
                }
                if let Err(error) = db.clear_account_data(owe_finder_removal) {
                    tracing::error!(error = %error, "sign-out stopped: the account rows could not be cleared");
                    return Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"));
                }
                Ok(())
            }
            // The guard's refusal is E2 with its code, so the caller can match it (spec §11).
            Err(error) if error.starts_with(&format!("{}: ", sign_out::copy::UNSENT_COPY_FAILED_CODE)) => Err(error),
            Err(error) => {
                tracing::error!(error = %error, "sign-out stopped: the local data could not be purged");
                Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"))
            }
        },
```

2. `purge_local_state_files(db, sync_root, kept)` passes `kept` to `purge_local_state_files_in(db, sync_root, &production_staging_dirs(), kept)`, which passes it to `purge_local_state_files_as(db, sync_root, staging_dirs, current_uid(), kept)`, which is:

```rust
fn purge_local_state_files_as(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_with(db, sync_root, staging_dirs, uid, kept, &|path| std::fs::remove_file(path))
}
```

3. `purge_local_state_files_with` is today's `purge_local_state_files_as` body with three changes, in this order:

```rust
fn purge_local_state_files_with(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<LocalStatePurgeSummary, String> {
    // Spec 2026-10-10 §5.2 step 8: given a `KeptSet` (macOS), the guard is the FIRST statement: one read, and a
    // refusal before any path is collected or deleted. Without one the purge is today's.
    let waiting_copies: std::collections::HashSet<PathBuf> = match kept {
        Some(kept) => {
            let guard = db
                .unkept_waiting_changes(kept)
                .map_err(|e| format!("check what the sign-out kept: {e}"))?;
            if !guard.unkept.is_empty() {
                tracing::error!(unkept = guard.unkept.len(), "sign-out stopped: a waiting change was not kept; nothing was deleted");
                return Err(sign_out::copy::coded(
                    sign_out::copy::UNSENT_COPY_FAILED_CODE,
                    sign_out::copy::E2_UNSENT_COPY_FAILED,
                ));
            }
            guard.waiting_payloads.into_iter().map(PathBuf::from).collect()
        }
        None => std::collections::HashSet::new(),
    };
    let mut paths: Vec<PathBuf> = db
        .local_state_file_paths()
        .map_err(|e| format!("list the local files: {e}"))?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let owned: std::collections::HashSet<PathBuf> = paths.iter().cloned().collect();
    paths.extend(
        orphan_staged_files(staging_dirs, uid)
            .into_iter()
            .filter(|path| !owned.contains(path)),
    );
    // W13: every other file first, the waiting writes' staged copies last, so a failure on any other file leaves every
    // waiting write as it was.
    let (waiting_last, others_first): (Vec<PathBuf>, Vec<PathBuf>) =
        paths.drain(..).partition(|path| waiting_copies.contains(path));
    let mut files_removed = 0usize;
    let mut files_skipped = 0usize;
    for group in [others_first, waiting_last] {
        let (removed, skipped) = remove_purge_paths(&group, remove)?;
        files_removed += removed;
        files_skipped += skipped;
    }
    let purge = db
        .purge_all_local_state()
        .map_err(|e| format!("purge operation queue and cache metadata: {e}"))?;
    // From here to the end, today's body as it is: the Windows placeholder step from the comment "Task 1538 Codex P1
    // (state_db.rs:1785 thread)" (`@184:lib.rs:4267`) on, and the summary it returns, with `files_removed` and
    // `files_skipped` now the two groups' sums.
}

/// Today's removal loop over one group: each path behind `is_disposable_cache_path`; a missing file is fine; any
/// other failure returns an error with no path in it. `(removed, skipped)`.
fn remove_purge_paths(
    paths: &[PathBuf],
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<(usize, usize), String> {
    let (mut removed, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    let mut first_failure: Option<std::io::ErrorKind> = None;
    for path in paths {
        if !is_disposable_cache_path(path) {
            skipped += 1;
            tracing::warn!(path = %path.display(), "purge skipped a path outside the known cache/staging roots");
            continue;
        }
        match remove(path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                failed += 1;
                first_failure.get_or_insert(error.kind());
                tracing::warn!(path = %path.display(), error = %error, "failed to remove a local file in the purge");
            }
        }
    }
    if failed > 0 {
        return Err(format!(
            "{failed} local file(s) could not be removed ({})",
            first_failure.map_or_else(|| "unknown".to_string(), |kind| kind.to_string())
        ));
    }
    Ok((removed, skipped))
}
```

4. Every existing caller passes `None` for the new parameter. List them, and change each:

```bash
cd $WT/src-tauri && grep -n -E "purge_local_state_files(_in|_as)?\(|purge_local_data_for_sign_out\(" src/lib.rs | grep -v -E "^\s*[0-9]+:\s*///|fn purge_local"
```

  The two production calls that pass a `KeptSet` come in Task 8 (`clear_session_impl`) and Task 14 (the R10 reset); until then they pass `None` too. A source pin that matches one of these calls by its text and breaks on the added argument is updated to the new text in this task, and named in the Notes.

- [ ] **Step 4: Run, expect PASS.** Filter `sign_out_keep_tests`: `ok. 6 passed` (2 from Task 2, 4 new). Then filter `purge_local_state_files` and `signout_teardown_tests`: the same counts as at the baseline run.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Run the guard after the removal loop: U14 red on "no staged copy was touched".
  2. Delete the kept folder on a guard refusal (`std::fs::remove_dir_all(folder)` before the `return`): U14 red on "the kept folder stays".
  3. Remove the files in one loop (`for group in [paths]`): U39 (a) red on "the waiting writes' copies go last".
  4. Never clear the marker: U39 (c) red.
  5. Wrap the guard's refusal in `SIGN_OUT_PURGE_FAILED` inside `purge_local_data_for_sign_out`: no test in this task sees that wrapper (U14 calls the purge below it). Task 8's `a_guard_refusal_reaches_the_caller_with_its_code` does; this mutation is run there.

- [ ] **Step 6: The task gate** with `<N>` = 6 and `<L>` = `L0 + 38`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/state_db.rs src-tauri/src/lib.rs
git commit -m "feat(macos): the sign-out purge refuses before any deletion unless every waiting save was kept (1887)" -- src-tauri/src/state_db.rs src-tauri/src/lib.rs
```

### Task 7: Close Finder accepts before the keep step (Q1, option A)

**Gated on Q1.** Start only once the lead has answered spec question 3. Under option B, skip this task: Task 20 adds option B's sentence to W1, and U45 is struck from the spec with that ruling named.

**Files:**
- Modify: `src-tauri/src/state_db.rs`: `FINDER_ACCEPTS_CLOSED_KEY`, `close_finder_accepts`, `finder_accepts_closed`; `AcceptOutcome::AcceptsClosed` (`@184:state_db.rs:1632-1650`); the check in `accept_finder_write` (`:3724-3743`); the reopen in `engine_start_repair` (`:5613-5620`)
- Modify: `src-tauri/src/engine_bridge.rs`: `AcceptsClosed` beside `StagedCopyGone` (`@184:engine_bridge.rs:756-770`); both `AcceptOutcome` matches (`:2633`, `:3047`)
- Modify: `src-tauri/src/ipc_socket.rs`: `write_refusal_category` (`@184:ipc_socket.rs:1452-1466`) and `write_outcome_response`'s arm beside `StagedCopyGone` (`:2573-2578`)
- Modify: `src-tauri/src/sign_out/keep.rs` (the close is `keep_unsent`'s first act)
- Modify: `src-tauri/src/lib.rs:3155-3164` (the comment on `stop_engine_for_sign_out` that says no request can be served after the stop)
- Test: `src-tauri/src/state_db.rs` (tests module), `src-tauri/src/ipc_socket.rs` (tests module), `src-tauri/src/sign_out/keep.rs`

**Interfaces:**
- Consumes: `accept_finder_write`, `FinderAccept`, `FinderAcceptKind`, `track_staged_payload`, `seed_own_row` (the existing `an_accept_whose_journal_row_is_gone_writes_nothing` test, `@184:state_db.rs:11912-11952`, is the pattern).
- Produces: `StateDb::close_finder_accepts(&self) -> rusqlite::Result<()>`, `StateDb::finder_accepts_closed(&self) -> rusqlite::Result<bool>`, `AcceptOutcome::AcceptsClosed`, `engine_bridge::AcceptsClosed`, the refusal category `"accepts_closed"`.

- [ ] **Step 1: Write the failing tests.** In `state_db.rs`'s tests, next to `an_accept_whose_journal_row_is_gone_writes_nothing`:

```rust
    fn accept_save(db: &StateDb, payload_path: &str) -> AcceptOutcome {
        db.accept_finder_write(
            &FinderAccept {
                op_id: "op-late",
                file_id: "late",
                kind: FinderAcceptKind::Modify { incoming_base: Some("1") },
                parent_id: None,
                target_path: Some("late.txt"),
                metadata_json: "{}",
                payload_path,
                size_bytes: 4,
                modified_at: 20,
                backup_source_key: None,
                now: 30,
            },
            &|_, _| Vec::new(),
        )
        .unwrap()
    }

    fn seeded_for_a_late_accept() -> (tempfile::TempDir, StateDb) {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "late", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("late").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        db.track_staged_payload("/staged/late", None, false).unwrap();
        (dir, db)
    }

    /// U45 (spec 2026-10-10, open question 3, option A).
    #[test]
    fn a_finder_accept_after_the_engine_stop_queues_nothing() {
        let (_dir, db) = seeded_for_a_late_accept();
        db.close_finder_accepts().unwrap();
        assert!(matches!(accept_save(&db, "/staged/late"), AcceptOutcome::AcceptsClosed));
        assert!(db.list_operations_for_file("late").unwrap().is_empty(), "no op");
        let unfinished: bool = db
            .hold_lock_for_test()
            .query_row("SELECT completed = 0 FROM staged_payloads WHERE path = '/staged/late'", [], |row| row.get(0))
            .unwrap();
        assert!(unfinished, "the journal row is left for the purge, or for the next start's sweep");
    }

    /// U45: an engine start, and a cleared account, take saves again.
    #[test]
    fn accepts_reopen_at_engine_start_and_with_the_cleared_account() {
        let (_dir, db) = seeded_for_a_late_accept();
        db.close_finder_accepts().unwrap();
        db.engine_start_repair().unwrap();
        assert!(!db.finder_accepts_closed().unwrap());
        assert!(matches!(accept_save(&db, "/staged/late"), AcceptOutcome::Queued { .. }));
        db.close_finder_accepts().unwrap();
        db.clear_account_data(false).unwrap();
        assert!(!db.finder_accepts_closed().unwrap());
    }

    /// U45: an accept whose transaction began first commits first, and a read after the close sees it. Two
    /// connections to one file, as the engine and the sign-out have.
    #[test]
    fn an_accept_already_running_commits_before_the_close() {
        for round in 0..50 {
            let dir = tempdir().unwrap();
            let path = dir.path().join("state.db");
            let engine = StateDb::open(&path).unwrap();
            let sign_out = StateDb::open(&path).unwrap();
            let (started, wait_started) = std::sync::mpsc::channel();
            let accept = std::thread::spawn(move || {
                let mut conn = engine.hold_lock_for_test();
                let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
                started.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(20));
                tx.execute("INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('late', 'upload_version', 'f')", [])
                    .unwrap();
                tx.commit().unwrap();
            });
            wait_started.recv().unwrap();
            sign_out.close_finder_accepts().unwrap();
            let rows = sign_out.waiting_rows().unwrap();
            accept.join().unwrap();
            assert!(rows.iter().any(|row| row.op_id == "late"), "round {round}");
        }
    }
```

In `ipc_socket.rs`'s tests:

```rust
    /// U45: the extension is told to retry, as for a copy an engine-start sweep removed.
    #[test]
    fn a_closed_accept_answers_write_retry_later() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let error = anyhow::Error::new(crate::engine_bridge::AcceptsClosed);
        assert_eq!(write_refusal_category(&error), "accepts_closed");
        let response = write_outcome_response("modify", &db, Err(anyhow::Error::new(crate::engine_bridge::AcceptsClosed)));
        assert!(matches!(response, IpcResponse::WriteRetryLater { .. }), "{response:?}");
    }
```

In `keep.rs`'s tests:

```rust
    /// U45: closing accepts is the keep step's first act, before its read, also when it then refuses.
    #[test]
    fn the_keep_step_closes_accepts_before_it_reads() {
        let f = Fixture::new();
        f.other("r1", "rename_file", "a");
        assert!(matches!(f.keep(Acknowledge::Nothing), Err(KeepFailure::Unsent)));
        assert!(f.db.finder_accepts_closed().unwrap());
        let source = include_str!("keep.rs");
        let body = &source[source.find("pub(crate) fn keep_unsent(").unwrap()..];
        let close = body.find("close_finder_accepts()").expect("the close");
        let read = body.find("WaitingChanges::read(").expect("the read");
        assert!(close < read, "accepts are closed before the read");
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `a_finder_accept_after`, `accepts_reopen`, `an_accept_already_running`, `a_closed_accept`, `closes_accepts_before`. Expected: compile errors, then (with stubs) the assertions.

- [ ] **Step 3: Implement.** In `state_db.rs`:

```rust
/// Spec 2026-10-10, open question 3 (option A): set by a sign-out's keep step after the engine stop, so a Finder accept
/// that outlived the stop queues nothing. Cleared by `engine_start_repair` and with the account's rows.
const FINDER_ACCEPTS_CLOSED_KEY: &str = "finder_accepts_closed";
```

`AcceptOutcome` gains, after `CopyGone`:

```rust
    /// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10, open question 3). Nothing was
    /// written; the write is retried, and macOS keeps the change until it is.
    AcceptsClosed,
```

In `accept_finder_write`, as the first statement inside its `BEGIN IMMEDIATE` transaction (before the `journalled` read):

```rust
        let closed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_state WHERE key = ?1)",
            params![FINDER_ACCEPTS_CLOSED_KEY],
            |row| row.get(0),
        )?;
        if closed {
            return Ok(AcceptOutcome::AcceptsClosed);
        }
```

In `impl StateDb`:

```rust
    /// One `BEGIN IMMEDIATE` transaction: it waits for an accept transaction already running, which therefore commits
    /// first, and every later accept reads the key in its own transaction and queues nothing.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn close_finder_accepts(&self) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![FINDER_ACCEPTS_CLOSED_KEY],
        )?;
        tx.commit()
    }

    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn finder_accepts_closed(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_state WHERE key = ?1)",
            params![FINDER_ACCEPTS_CLOSED_KEY],
            |row| row.get(0),
        )
    }
```

In `engine_start_repair`, right after `claims_cleared`: `tx.execute("DELETE FROM sync_state WHERE key = ?1", params![FINDER_ACCEPTS_CLOSED_KEY])?;` (`clear_account_data` already deletes every `sync_state` row).

In `engine_bridge.rs`, beside `StagedCopyGone`:

```rust
/// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10, open question 3). Nothing was queued; the
/// reply is `WriteRetryLater`, so macOS keeps the change and retries. Path-free: the text reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct AcceptsClosed;

impl std::fmt::Display for AcceptsClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Beebeeb is signing out; this write was not queued")
    }
}

impl std::error::Error for AcceptsClosed {}
```

and in both matches (`@184:engine_bridge.rs:2633`, `:3047`): `crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),`. In `ipc_socket.rs`, `write_refusal_category` gains `if error.is::<crate::engine_bridge::AcceptsClosed>() { return "accepts_closed"; }` after the `StagedCopyGone` check, and `write_outcome_response` gains, after the `StagedCopyGone` arm:

```rust
        // Spec 2026-10-10, open question 3: a sign-out closed accepts after its engine stop. Nothing was queued; the
        // extension reports a transient error, so the system keeps the change and hands it over again.
        Err(e) if e.is::<crate::engine_bridge::AcceptsClosed>() => {
            log_refused_write(op, write_refusal_category(&e));
            IpcResponse::WriteRetryLater { message: e.to_string() }
        }
```

In `keep_unsent`, the first statement:

```rust
    // Open question 3 (option A): a Finder accept can outlive the engine stop (`spawn_blocking`). Closing accepts in one
    // IMMEDIATE transaction lets one already running commit first, so the read below sees it, and refuses every later one.
    inputs.db.close_finder_accepts().map_err(|_| KeepFailure::Copy("close_accepts"))?;
```

Rewrite the comment above `stop_engine_for_sign_out` (`@184:lib.rs:3159-3164`) so it no longer says no request can be served after the stop: "Then upload staging: every handed-over copy is removed. A request that arrives later reaches no daemon and fails transiently. One accept may still be running in `spawn_blocking`: the keep step closes accepts before it reads the queue (spec 2026-10-10 §5.2 step 6)."

- [ ] **Step 4: Run, expect PASS.** The five filters: `ok. 1 passed` each; then filter `sign_out::keep::`: `ok. 13 passed`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Remove the check in `accept_finder_write`: `a_finder_accept_after…` red (`Queued`).
  2. Remove the reopen in `engine_start_repair`: `accepts_reopen…` red on the first `assert!`.
  3. Make `close_finder_accepts` an in-memory flag that takes no SQLite lock (a `static CLOSED: AtomicBool` set and read instead of the key): `an_accept_already_running…` red, because the read runs before the running accept commits.
  4. Move `close_finder_accepts()` below `WaitingChanges::read(` in `keep_unsent`: `the_keep_step_closes_accepts_before_it_reads` red.
  5. Map `AcceptsClosed` to `IpcResponse::Error`: `a_closed_accept…` red.

- [ ] **Step 6: The task gate** with `<N>` = 7 and `<L>` = `L0 + 43`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
git commit -m "feat(macos): a sign-out closes Finder accepts before it reads what to keep (1887)" -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
```

### Task 8: The ask snapshot, the gate and the sign-out

**Files:**
- Create: `src-tauri/src/sign_out/snapshot.rs`
- Modify: `src-tauri/src/sign_out/mod.rs` (`SignOutState`)
- Modify: `src-tauri/src/lib.rs`: `AppState` (`@184:lib.rs:248-309` and its `Default` at `:309`); `SignedOut`, `SignOutFailure` (`:2753-2797`); `clear_session_impl` (`:2823-3129`); `clear_session` (`:3311-3324`); the menu's call (`:13998`); every test call of `clear_session_impl(`
- Test: `src-tauri/src/sign_out/snapshot.rs`; `src-tauri/src/lib.rs` `mod sign_out_keep_tests`; a `#[cfg(all(test, not(target_os = "macos")))]` test beside it for U32's other half

**Interfaces:**
- Consumes: `WaitingChanges` (Task 1); `keep_unsent`, `KeepInputs`, `Acknowledge`, `KeptSet` (Task 5); `kept_names::{event_folder_name, kept_root_for_this_app}` (Task 2); `kept_record::unfinished_at` (Task 4); `copy::*` (Task 5); the purge's `kept` parameter (Task 6).
- Produces:
  - `snapshot::{AskCounts, AskSnapshot, SnapshotStore, GateRefusal}`; `AskCounts` serializes as `{ askId, files, copiedFiles, others, missingFiles, earlierFolders }`; `AskSnapshot::of(&WaitingChanges, owner: &str, earlier_folders: Vec<String>)`; `SnapshotStore::{record, newest_ask_id, drop_owned_by, clear, gate}`.
  - `sign_out::SignOutState { snapshots: Arc<SnapshotStore> }`, `AppState.sign_out: sign_out::SignOutState`.
  - `lib.rs` (macOS): `fn count_ask(owner: &str) -> Result<AskSnapshot, String>`, `fn gate_sign_out(state: &AppState, keep_unsent: Option<&str>) -> Result<Option<AskSnapshot>, SignOutFailure>`, `fn keep_for_sign_out(accepted: Option<&AskSnapshot>) -> Result<Option<KeptSet>, String>`, `fn sign_out_kept_root() -> Result<PathBuf, String>`; `struct KeptUnsent { folder: String, files: usize }`; `SignedOut.kept_unsent` and `SignOutFailure.kept_unsent: Option<KeptUnsent>`.
  - `clear_session_impl(state, forget_email, keep_unsent: Option<&str>)`; `clear_session(…, keep_unsent: Option<KeepUnsentArg>)` with `#[derive(serde::Deserialize)] #[serde(rename_all = "camelCase")] struct KeepUnsentArg { ask_id: String }`.
  - Test seam (test builds only): `tokio::task_local! { static SIGN_OUT_SEAM: SignOutSeam }` with `type SignOutSeam = Arc<dyn Fn(&'static str) -> Result<(), String> + Send + Sync>`, called at `"before_keep"`, `"after_keep"` and `"after_purge"`.

- [ ] **Step 1: Write the failing tests.** In `snapshot.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign_out::waiting::{OtherChange, WaitingChanges, WaitingFile, WaitingWrite, WriteState};
    use crate::state_db::WriteOrigin;

    fn waiting(files: &[(&str, &[(&str, WriteState)])], others: &[&str]) -> WaitingChanges {
        WaitingChanges {
            files: files
                .iter()
                .map(|(file_id, writes)| WaitingFile {
                    file_id: file_id.to_string(),
                    path: None,
                    writes: writes
                        .iter()
                        .enumerate()
                        .map(|(i, (op_id, state))| WaitingWrite {
                            op_id: op_id.to_string(),
                            payload_path: None,
                            origin: WriteOrigin::Minted,
                            queue_order: i as i64,
                            live: true,
                            state: *state,
                        })
                        .collect(),
                })
                .collect(),
            others: others.iter().map(|op_id| OtherChange { op_id: op_id.to_string(), live: true }).collect(),
        }
    }

    /// U37, the gate's comparison (Task 8's sign-out tests add the keep step's snapshot).
    #[test]
    fn the_gate_compares_the_queue_with_the_snapshot() {
        use WriteState::*;
        let asked = waiting(&[("f1", &[("w1", Copied)])], &["r1"]);
        let store = SnapshotStore::default();
        assert_eq!(store.gate(&asked, None), Err(GateRefusal), "changes wait and no keepUnsent");
        let snapshot = AskSnapshot::of(&asked, "macos-settings", Vec::new());
        let id = snapshot.ask_id().to_string();
        store.record(snapshot);
        assert_eq!(store.gate(&asked, Some("stale")), Err(GateRefusal));
        assert_eq!(store.newest_ask_id().as_deref(), Some(id.as_str()), "a refusal leaves the store as it was");
        let new_file = waiting(&[("f1", &[("w1", Copied)]), ("f2", &[("w2", Copied)])], &["r1"]);
        assert_eq!(store.gate(&new_file, Some(&id)), Err(GateRefusal), "a waiting file it did not hold");
        let new_other = waiting(&[("f1", &[("w1", Copied)])], &["r1", "r2"]);
        assert_eq!(store.gate(&new_other, Some(&id)), Err(GateRefusal), "an other change it did not hold");
        let new_missing = waiting(&[("f1", &[("w1", Copied), ("w9", Missing)])], &["r1"]);
        assert_eq!(store.gate(&new_missing, Some(&id)), Err(GateRefusal), "a missing write it did not hold");
        let autosave = waiting(&[("f1", &[("w1", Covered), ("w2", Copied)])], &["r1"]);
        let accepted = store.gate(&autosave, Some(&id)).expect("a newer write to a held file never refuses");
        assert_eq!(accepted.map(|s| s.ask_id().to_string()), Some(id));
        assert_eq!(store.newest_ask_id(), None, "the gate takes the snapshot out of the store");
        assert_eq!(store.gate(&WaitingChanges::default(), None), Ok(None), "nothing waits, no keepUnsent: no refusal");
    }

    #[test]
    fn earlier_folders_ride_only_an_ask_that_shows_a_missing_write() {
        use WriteState::*;
        let folders = vec!["/kept/one".to_string()];
        let none_missing = AskSnapshot::of(&waiting(&[("f1", &[("w1", Copied)])], &[]), "o", folders.clone());
        assert!(none_missing.counts.earlier_folders.is_empty());
        let missing = AskSnapshot::of(&waiting(&[("f1", &[("w1", Missing)])], &[]), "o", folders.clone());
        assert_eq!(missing.counts.earlier_folders, folders);
        let json = serde_json::to_value(&missing.counts).unwrap();
        for key in ["askId", "files", "copiedFiles", "others", "missingFiles", "earlierFolders"] {
            assert!(json.get(key).is_some(), "{key} in {json}");
        }
    }
}
```

In `lib.rs`'s `sign_out_keep_tests`, the sign-out tests. They share the scratch state dir, so each runs inside `with_the_shared_state_dir`:

```rust
    use crate::sign_out::snapshot::AskSnapshot;
    use crate::sign_out::waiting::WaitingChanges;

    fn state_db_in_scratch() -> state_db::StateDb {
        let dir = state_paths::init_for_test();
        state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&dir)).unwrap()
    }

    /// The scratch database as a completed sign-out leaves it: no rows, accepts open again (Task 7). A test that ends
    /// with rows left, or after a keep step that refused, calls it last, so the next test starts clean.
    fn reset_scratch(db: &state_db::StateDb) {
        db.purge_all_local_state().unwrap();
        db.clear_account_data(false).unwrap();
    }

    /// `signout_teardown_tests::test_account` (`@184:lib.rs:18240-18243`), which that module keeps private.
    fn test_account(state: &AppState, id: &str) -> Arc<crate::account::AccountRuntime> {
        synthesize_single_account(state, AccountId(id.to_string()));
        state.active_account().expect("synthesized account resolves")
    }

    /// Records the reconciler's removals and triggers, and confirms every removal and Lock.
    fn recording_finder(state: &AppState) -> Arc<std::sync::Mutex<Vec<String>>> {
        use crate::finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
        let (handle, mut rx) =
            FinderSetupHandle::for_test(FinderSetupView::initial(crate::finder_setup::launch_location::LaunchLocation::Applications));
        let _ = state.finder_setup.set(handle);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    Event::Remove { ack, .. } => {
                        log.lock().unwrap().push("remove".to_string());
                        let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
                    }
                    Event::Lock { ack } => {
                        let _ = ack.send(());
                    }
                    Event::Trigger(trigger) => log.lock().unwrap().push(format!("{trigger:?}")),
                    _ => {}
                }
            }
        });
        seen
    }

    fn queue_write(db: &state_db::StateDb, op_id: &str, file_id: &str, bytes: &[u8]) -> PathBuf {
        let staged = std::env::temp_dir().join(format!("bb-1887-{}-{op_id}", std::process::id()));
        std::fs::write(&staged, bytes).unwrap();
        db.hold_lock_for_test()
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO files (file_id, path) VALUES ('{file_id}', '{file_id}.bin');
                 INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                 VALUES ('{op_id}', 'upload_version', '{file_id}', '{}', '{op_id}', 'minted');",
                staged.display()
            ))
            .unwrap();
        staged
    }

    fn queue_rename(db: &state_db::StateDb, op_id: &str) {
        db.hold_lock_for_test()
            .execute("INSERT INTO operation_queue (op_id, kind, file_id) VALUES (?1, 'rename_file', 'f')", rusqlite::params![op_id])
            .unwrap();
    }

    fn ask(state: &AppState, owner: &str) -> String {
        let snapshot = count_ask(owner).unwrap();
        let id = snapshot.ask_id().to_string();
        state.sign_out.snapshots.record(snapshot);
        id
    }

    fn signed_out_state() -> (AppState, Arc<crate::account::AccountRuntime>) {
        let state = AppState::default();
        let acct = test_account(&state, "signout-1887");
        set_auth_present(&state, false);
        (state, acct)
    }

    async fn sign_out_with(state: &AppState, keep: Option<&str>, seam: SignOutSeam) -> Result<SignedOut, SignOutFailure> {
        SIGN_OUT_SEAM.scope(seam, clear_session_impl(state, false, keep)).await
    }

    fn no_seam() -> SignOutSeam {
        Arc::new(|_| Ok(()))
    }

    /// U6.
    #[tokio::test]
    async fn clear_session_refuses_while_changes_wait() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-u6");
            let events = recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session { token: "t".into(), master_key: [7u8; 32], email: Some("a@beebeeb.io".into()) });
            set_auth_present(&state, true);
            let failure = sign_out_with(&state, None, no_seam()).await.unwrap_err();
            assert!(failure.message.starts_with("unsent_changes: "), "{}", failure.message);
            assert_eq!(db.waiting_rows().unwrap().len(), 1, "the queue is intact");
            assert!(staged.exists(), "the staged copy is intact");
            assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
            assert!(events.lock().unwrap().iter().all(|event| event != "remove"), "the reconciler got no removal");
            reset_scratch(&db);
        })
        .await;
    }

    /// U7.
    #[test]
    fn the_gate_comes_before_anything_changes() {
        let production = super::finder_setup_command_tests::production_source();
        let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        let gate = clear.find("gate_sign_out(").expect("the gate");
        for later in [
            "forget_last_signed_in_email(",
            "end_sessions_in_flight(",
            "finder_remove_for(",
            "take_slot_and_stop_engine_for_sign_out(",
            "purge_local_data_for_sign_out(",
        ] {
            assert!(gate < clear.find(later).unwrap_or_else(|| panic!("{later}")), "the gate comes before {later}");
        }
    }

    /// U9: the gate counted nothing; a write that arrives before the keep step is kept and counted.
    #[tokio::test]
    async fn a_write_that_arrives_after_the_count_is_kept_too() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    let db = state_db_in_scratch();
                    queue_write(&db, "late", "late", b"late bytes");
                }
                Ok(())
            });
            let done = sign_out_with(&state, None, seam).await.unwrap();
            let kept = done.kept_unsent.expect("a kept folder");
            assert_eq!(kept.files, 1);
            assert_eq!(std::fs::read(Path::new(&kept.folder).join("late.bin")).unwrap(), b"late bytes");
            assert_eq!(db.waiting_rows().unwrap().len(), 0, "the queue is empty");
        })
        .await;
    }

    /// U10 (spec, amended for `184ac962`).
    #[test]
    fn nothing_that_deletes_runs_between_stop_keep_and_purge() {
        let production = super::finder_setup_command_tests::production_source();
        let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        let slot = clear.find("let mut held_engine_slot = if already_signed_out {").expect("the stop expression");
        let second_stop = clear.rfind("take_slot_and_stop_engine_for_sign_out(").unwrap();
        let close = slot + clear[slot..].find("\n    };\n").expect("the expression's end");
        let keep = clear.find("keep_for_sign_out(").expect("the keep call");
        let purge = clear.find("purge_local_data_for_sign_out(").unwrap();
        assert!(second_stop < close && close < keep && keep < purge, "stop arms, then `}};`, then keep, then purge");
        let between = &clear[second_stop..purge];
        for denied in [
            "remove_file", "remove_dir", "purge_all_local_state(", "purge_macos_hydrate_cache(",
            "clear_session_holding_slot(", "clear_keychain_session(", "forget_last_signed_in_email(", "start_engine", "KeysArrived",
        ] {
            assert!(!between.contains(denied), "{denied} between the stop and the purge");
        }
    }

    /// U13, the sign-out half: a failed copy leaves the keys in memory and asks the reconciler to restart sync.
    #[tokio::test]
    async fn a_failed_copy_keeps_the_keys_and_restarts_sync() {
        with_the_shared_state_dir(async {
            use std::os::unix::fs::PermissionsExt;
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
            let state = AppState::default();
            let acct = test_account(&state, "signout-1887-u13");
            let events = recording_finder(&state);
            *acct.session.lock().unwrap() = Some(super::Session { token: "t".into(), master_key: [7u8; 32], email: None });
            set_auth_present(&state, true);
            let id = ask(&state, "macos-settings");
            let failure = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(failure.message.starts_with("unsent_copy_failed: "), "{}", failure.message);
            assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(events.lock().unwrap().iter().any(|event| event == "KeysArrived"), "sync restarts: {:?}", events.lock().unwrap());
            assert_eq!(db.waiting_rows().unwrap().len(), 1);
            reset_scratch(&db);
        })
        .await;
    }

    /// U29 (b): a copy deleted before the call is a missing write the snapshot did not hold.
    #[tokio::test]
    async fn a_copy_missing_before_the_call_asks_again() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            std::fs::remove_file(&staged).unwrap();
            let failure = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(failure.message.starts_with("unsent_changes: "), "the gate refuses, the ask is shown again");
            let fresh = count_ask("macos-settings").unwrap();
            assert_eq!(fresh.counts.missing_files, 1);
            reset_scratch(&db);
        })
        .await;
    }

    /// U30: the record is on disk before the purge, and only the keep step writes it.
    #[tokio::test]
    async fn the_kept_folder_is_recorded_before_the_purge() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            let seam: SignOutSeam = Arc::new(|point| if point == "after_purge" { Err("aborted by the test".into()) } else { Ok(()) });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            let folder = failure.kept_unsent.expect("the failure carries the folder").folder;
            let config = DesktopConfig::load().unwrap();
            assert_eq!(config.kept_unsent_folder.as_deref(), Some(folder.as_str()));
            assert!(config.kept_unsent_unfinished.is_empty(), "the purge cleared the rows, so it cleared the marker");
        })
        .await;
        let production = super::finder_setup_command_tests::production_source();
        let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        assert!(!clear.contains("save_kept_unsent_durably("), "the Err arm writes neither key; only the keep step does");
    }

    /// U32, the macOS half: every new step is macOS only.
    #[test]
    fn the_unsent_gate_is_macos_only() {
        let production = super::finder_setup_command_tests::production_source();
        let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
        for call in ["gate_sign_out(", "keep_for_sign_out("] {
            let at = clear.find(call).unwrap_or_else(|| panic!("{call}"));
            let before = &clear[..at];
            let cfg = before.rfind("#[cfg(").unwrap();
            assert!(before[cfg..].starts_with("#[cfg(target_os = \"macos\")]"), "{call} is under cfg(macos)");
        }
        for definition in ["fn gate_sign_out(", "fn keep_for_sign_out(", "fn count_ask("] {
            let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
            let before = &production[..at];
            assert!(before[before.rfind("#[cfg(").unwrap()..].starts_with("#[cfg(target_os = \"macos\")]"), "{definition}");
        }
    }

    /// U37, the keep step's snapshot: the one the gate accepted, by value, never a newer one.
    #[tokio::test]
    async fn the_keep_step_uses_the_snapshot_the_gate_accepted() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_rename(&db, "r1");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            let store = state.sign_out.snapshots.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    queue_rename(&state_db_in_scratch(), "r2");
                    store.record(count_ask("macos-settings").unwrap()); // S′ holds both renames
                }
                Ok(())
            });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            assert!(failure.message.starts_with("unsent_changes: "), "{}", failure.message);
            assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing purged");
            // Second run: S′ recorded without changing the queue; the sign-out completes and empties the store.
            db.hold_lock_for_test().execute("DELETE FROM operation_queue WHERE op_id = 'r2'", []).unwrap();
            let id = ask(&state, "macos-settings");
            let store = state.sign_out.snapshots.clone();
            let seam: SignOutSeam = Arc::new(move |point| {
                if point == "before_keep" {
                    store.record(count_ask("macos-settings").unwrap());
                }
                Ok(())
            });
            sign_out_with(&state, Some(&id), seam).await.unwrap();
            assert_eq!(state.sign_out.snapshots.newest_ask_id(), None, "a completed sign-out empties the store");
        })
        .await;
    }

    /// U41: entry point 7, the already-signed-out arm, keeps before it purges.
    #[tokio::test]
    async fn an_already_signed_out_sign_out_keeps_before_it_purges() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"already signed out");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
            assert_eq!(done.outcome, SignOutOutcome::NotSignedIn, "the already-signed-out arm ran");
            let kept = done.kept_unsent.unwrap();
            assert_eq!(std::fs::read(Path::new(&kept.folder).join("f1.bin")).unwrap(), b"already signed out");
            assert!(db.waiting_rows().unwrap().is_empty());
        })
        .await;
    }

    /// U14 through the sign-out: the guard's refusal reaches the caller with its code, and the folder rides it.
    #[tokio::test]
    async fn a_guard_refusal_reaches_the_caller_with_its_code() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            let seam: SignOutSeam = Arc::new(|point| {
                if point == "after_keep" {
                    queue_write(&state_db_in_scratch(), "late", "late", b"late"); // only a test seam can add one now
                }
                Ok(())
            });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            assert!(failure.message.starts_with("unsent_copy_failed: "), "never wrapped: {}", failure.message);
            assert!(failure.kept_unsent.is_some(), "the kept folder rides the failure");
            assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing deleted");
            reset_scratch(&db);
        })
        .await;
    }

    /// Review Focus 1.
    #[tokio::test]
    async fn a_second_sign_out_with_the_same_ask_keeps_nothing_twice() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"once");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "macos-settings");
            let first = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
            assert!(first.kept_unsent.is_some());
            queue_write(&db, "w2", "f2", b"again"); // something waits again, so the stale ask matters
            let second = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(second.message.starts_with("unsent_changes: "), "{}", second.message);
            assert_eq!(second.kept_unsent, None, "no second folder");
            reset_scratch(&db);
        })
        .await;
    }

    /// U39 (b), the ask's half: an interrupted sign-out's folder is named when the ask shows a missing write.
    #[tokio::test]
    async fn the_ask_names_the_earlier_folders_only_with_a_missing_write() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            let staged = queue_write(&db, "w1", "f1", b"bytes");
            DesktopConfig::update(|cfg| {
                cfg.kept_unsent_unfinished = vec!["/kept/earlier".into()];
                (true, ())
            })
            .unwrap();
            assert!(count_ask("o").unwrap().counts.earlier_folders.is_empty(), "no missing write: no A3e");
            std::fs::remove_file(&staged).unwrap();
            assert_eq!(count_ask("o").unwrap().counts.earlier_folders, ["/kept/earlier"]);
            DesktopConfig::update(|cfg| {
                cfg.kept_unsent_unfinished.clear();
                (true, ())
            })
            .unwrap();
            reset_scratch(&db);
        })
        .await;
    }
```

Off macOS, beside the module:

```rust
/// U32, the other half (spec 2026-10-10 §10): Linux and Windows keep today's sign-out. Runs in the public CI only.
#[cfg(all(test, not(target_os = "macos")))]
mod sign_out_unchanged_elsewhere_tests {
    use super::*;

    #[tokio::test]
    async fn the_unsent_gate_is_macos_only() {
        with_the_shared_state_dir(async {
            let dir = state_paths::init_for_test();
            let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
            db.hold_lock_for_test()
                .execute("INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('r1', 'rename_file', 'f')", [])
                .unwrap();
            let state = AppState::default();
            synthesize_single_account(&state, AccountId("signout-1887-elsewhere".to_string()));
            set_auth_present(&state, false);
            let result = clear_session_impl(&state, false, None).await;
            #[cfg(target_os = "linux")]
            {
                assert!(result.is_ok(), "Linux purges as before: {result:?}");
                assert!(db.waiting_rows().unwrap().is_empty());
            }
            #[cfg(target_os = "windows")]
            {
                let _ = &result;
                assert!(!format!("{result:?}").contains("unsent_changes"), "Windows keeps its own preflight");
            }
        })
        .await;
    }
}
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `sign_out::snapshot` and `sign_out_keep_tests`. Expected: compile errors (the new names, the third argument).

- [ ] **Step 3: Implement.** `src-tauri/src/sign_out/snapshot.rs` above its tests:

```rust
//! Spec 2026-10-10 §2 "Ask snapshot" and W5 (task 1887): what one ask showed the person, held by Rust, never by the
//! webview, and the gate's comparison of the queue with it.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use super::waiting::WaitingChanges;

/// What the webview renders (§5.1, the contract).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskCounts {
    pub ask_id: String,
    pub files: usize,
    pub copied_files: usize,
    pub others: usize,
    pub missing_files: usize,
    /// §7.3: every interrupted sign-out's folder, oldest first, only when the ask shows a missing write.
    pub earlier_folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskSnapshot {
    pub counts: AskCounts,
    /// The label of the window it was counted for.
    pub owner: String,
    pub files: BTreeSet<String>,
    pub others: BTreeSet<String>,
    pub missing: BTreeSet<String>,
}

impl AskSnapshot {
    pub fn of(waiting: &WaitingChanges, owner: &str, earlier_folders: Vec<String>) -> Self {
        let counts = waiting.counts();
        Self {
            counts: AskCounts {
                ask_id: uuid::Uuid::new_v4().to_string(),
                files: counts.files,
                copied_files: counts.copied_files,
                others: counts.others,
                missing_files: counts.missing_files,
                earlier_folders: if counts.missing_files > 0 { earlier_folders } else { Vec::new() },
            },
            owner: owner.to_string(),
            files: waiting.file_ids(),
            others: waiting.other_op_ids(),
            missing: waiting.missing_op_ids(),
        }
    }

    pub fn ask_id(&self) -> &str {
        &self.counts.ask_id
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.others.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateRefusal;

/// The one newest snapshot (§2): a newer count replaces it; the gate takes it out; a close of its owner drops it; a
/// completed sign-out empties the store.
#[derive(Default)]
pub struct SnapshotStore {
    newest: Mutex<Option<AskSnapshot>>,
}

impl SnapshotStore {
    fn lock(&self) -> MutexGuard<'_, Option<AskSnapshot>> {
        self.newest.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn record(&self, snapshot: AskSnapshot) {
        *self.lock() = Some(snapshot);
    }

    pub fn newest_ask_id(&self) -> Option<String> {
        self.lock().as_ref().map(|snapshot| snapshot.ask_id().to_string())
    }

    /// A close or reload of `label` (re-review I6). Another window's snapshot is left alone.
    pub fn drop_owned_by(&self, label: &str) -> bool {
        let mut newest = self.lock();
        if newest.as_ref().is_some_and(|snapshot| snapshot.owner == label) {
            *newest = None;
            return true;
        }
        false
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// W5. Refuses when changes wait and no snapshot is named; when the named one is not the newest; or when the queue
    /// holds a waiting file, an other change or a missing write it did not hold. A newer write to a held file does
    /// not refuse (the autosave case). Accepting takes the snapshot out of the store and hands it over by value; a
    /// refusal leaves the store as it was.
    pub fn gate(&self, waiting: &WaitingChanges, keep_unsent: Option<&str>) -> Result<Option<AskSnapshot>, GateRefusal> {
        let mut newest = self.lock();
        let Some(ask_id) = keep_unsent else {
            return if waiting.is_empty() { Ok(None) } else { Err(GateRefusal) };
        };
        let Some(snapshot) = newest.as_ref().filter(|snapshot| snapshot.ask_id() == ask_id) else {
            return Err(GateRefusal);
        };
        let held = waiting.file_ids().is_subset(&snapshot.files)
            && waiting.other_op_ids().is_subset(&snapshot.others)
            && waiting.missing_op_ids().is_subset(&snapshot.missing);
        if !held {
            return Err(GateRefusal);
        }
        Ok(newest.take())
    }
}
```

`sign_out/mod.rs` gains `pub(crate) mod snapshot;` and:

```rust
/// The sign-out flow's state in `AppState` (spec 2026-10-10 §5.1). Tasks 11-13 add the send phase, the late results
/// and the flow request.
#[derive(Default)]
pub(crate) struct SignOutState {
    pub snapshots: std::sync::Arc<snapshot::SnapshotStore>,
}
```

`AppState` gains `pub sign_out: sign_out::SignOutState,` and its `Default` gains `sign_out: sign_out::SignOutState::default(),`.

In `lib.rs`, near `SignedOut`:

```rust
/// Spec 2026-10-10 §5.2 step 10: the folder the keep step made and its file count. Never logged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeptUnsent {
    folder: String,
    files: usize,
}
```

`SignedOut` and `SignOutFailure` each gain `kept_unsent: Option<KeptUnsent>`; `SignedOut::confirmed` and both `From` impls set it to `None`; every other struct literal the compiler names gains `kept_unsent: None` except the ones below.

The sign-out's macOS helpers, next to `purge_local_data_for_sign_out`:

```rust
/// Spec 2026-10-10 §5.1 step 1 without the send conditions (Task 11 adds them): one read; the earlier folders only
/// when the ask shows a missing write. A failed read is E3, never a 0.
#[cfg(target_os = "macos")]
fn count_ask(owner: &str) -> Result<sign_out::snapshot::AskSnapshot, String> {
    let e3 = || sign_out::copy::coded(sign_out::copy::UNSENT_COUNT_FAILED_CODE, sign_out::copy::E3_UNSENT_COUNT_FAILED);
    let waiting = read_waiting_changes().map_err(|_| e3())?;
    let earlier = if waiting.counts().missing_files > 0 {
        DesktopConfig::path().and_then(|path| sign_out::kept_record::unfinished_at(&path)).unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(sign_out::snapshot::AskSnapshot::of(&waiting, owner, earlier))
}

/// The queue as the sign-out counts it, from the app's state dir. No `state.db` yet: nothing waits.
#[cfg(target_os = "macos")]
fn read_waiting_changes() -> Result<sign_out::waiting::WaitingChanges, String> {
    match state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)? {
        Some(db) => sign_out::waiting::WaitingChanges::read(&db),
        None => Ok(sign_out::waiting::WaitingChanges::default()),
    }
}

/// W5, the gate: one read compared with the snapshot `keep_unsent` names.
#[cfg(target_os = "macos")]
fn gate_sign_out(state: &AppState, keep_unsent: Option<&str>) -> Result<Option<sign_out::snapshot::AskSnapshot>, SignOutFailure> {
    let waiting = read_waiting_changes().map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(sign_out::copy::UNSENT_COUNT_FAILED_CODE, sign_out::copy::E3_UNSENT_COUNT_FAILED))
    })?;
    state.sign_out.snapshots.gate(&waiting, keep_unsent).map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(sign_out::copy::UNSENT_CHANGES_CODE, sign_out::copy::E1_UNSENT_CHANGES))
    })
}

/// Where the kept folder goes: the container's `Documents/Changes not sent` (§7.1). A test build writes under its
/// per-process sandbox, never the person's folders.
#[cfg(target_os = "macos")]
fn sign_out_kept_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        crate::test_sandbox::dir("kept").map(|dir| dir.join("Changes not sent")).map_err(|e| e.to_string())
    }
    #[cfg(not(test))]
    {
        sign_out::kept_names::kept_root_for_this_app().ok_or_else(|| "no folder for kept changes".to_string())
    }
}

/// §5.2 step 7: the keep step on the app's state dir. `None`: no `state.db` yet, nothing to keep.
#[cfg(target_os = "macos")]
fn keep_for_sign_out(accepted: Option<&sign_out::snapshot::AskSnapshot>) -> Result<Option<sign_out::kept_set::KeptSet>, String> {
    use sign_out::keep::{Acknowledge, KeepInputs, keep_unsent};
    let failed = || sign_out::copy::coded(sign_out::copy::UNSENT_COPY_FAILED_CODE, sign_out::copy::E2_UNSENT_COPY_FAILED);
    let Some(db) = state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)? else {
        return Ok(None);
    };
    let root = sign_out_kept_root().map_err(|_| failed())?;
    let config = DesktopConfig::path().map_err(|_| failed())?;
    let acknowledge = match accepted {
        Some(snapshot) => Acknowledge::Asked { others: &snapshot.others, missing: &snapshot.missing },
        None => Acknowledge::Nothing,
    };
    keep_unsent(KeepInputs {
        db: &db,
        acknowledge,
        root: &root,
        event_name: sign_out::kept_names::event_folder_name(chrono::Local::now()),
        config_path: &config,
        clone: &sign_out::kept_writer::SystemClone,
        config_fs: &config::RealConfigFs,
    })
    .map(Some)
    .map_err(|failure| {
        if let sign_out::keep::KeepFailure::Copy(reason) = &failure {
            tracing::error!(reason, "sign-out stopped: the waiting changes could not be kept; nothing was purged");
        }
        failure.message()
    })
}

/// Test builds only: a hook between the steps of `clear_session_impl`, per test task (`SIGN_OUT_SEAM.scope`). An
/// `Err` stops the sign-out at that point with that message.
#[cfg(test)]
type SignOutSeam = Arc<dyn Fn(&'static str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
tokio::task_local! {
    static SIGN_OUT_SEAM: SignOutSeam;
}

#[cfg(test)]
fn sign_out_seam(point: &'static str) -> Result<(), String> {
    SIGN_OUT_SEAM.try_with(|seam| seam(point)).unwrap_or(Ok(()))
}
```

In `clear_session_impl`:
1. Its signature becomes `async fn clear_session_impl(state: &AppState, forget_email: bool, keep_unsent: Option<&str>) -> Result<SignedOut, SignOutFailure>`. Its first statements, before `let acct = state.active_account()?;`:

```rust
    // Spec 2026-10-10 W5: the gate runs first, before anything changes. A refusal changes nothing.
    #[cfg(target_os = "macos")]
    let accepted = gate_sign_out(state, keep_unsent)?;
    #[cfg(not(target_os = "macos"))]
    let _ = keep_unsent;
```

2. Right after the `};` that closes `let mut held_engine_slot = if already_signed_out { … } else { … };` (`@184:lib.rs:2945`), before the Windows block:

```rust
    // Spec 2026-10-10 §5.2 step 7 (W6): after the confirmed stop of either arm above, before the purge. Nothing between
    // here and the purge deletes a file, writes the queue, clears a session or starts an engine (U10).
    #[cfg(test)]
    sign_out_seam("before_keep").map_err(&kept_with)?;
    #[cfg(target_os = "macos")]
    let kept = keep_for_sign_out(accepted.as_ref()).map_err(&kept_with)?;
    #[cfg(not(target_os = "macos"))]
    let kept: Option<sign_out::kept_set::KeptSet> = None;
    let kept_unsent = kept.as_ref().and_then(|kept| {
        kept.folder.as_ref().map(|folder| KeptUnsent { folder: folder.to_string_lossy().into_owned(), files: kept.files })
    });
    // §5.2 step 10: from here on every failure carries our folder too.
    let kept_with = |message: String| SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: kept_unsent.clone(),
    };
    #[cfg(test)]
    sign_out_seam("after_keep").map_err(&kept_with)?;
```

  The first `kept_with` (`@184:lib.rs:2854-2857`) sets `kept_unsent: None`.

3. The purge call passes `kept.as_ref()`: `purge_local_data_for_sign_out(state_paths::beebeeb_state_dir(), cfg_sync_root.as_deref(), owe_removal, kept.as_ref())`, then `#[cfg(test)] sign_out_seam("after_purge").map_err(&kept_with)?;`.
4. Both `Ok(SignedOut { … })` returns gain `kept_unsent: kept_unsent.clone()`, and just before each: `state.sign_out.snapshots.clear();` (re-review N3: a completed sign-out empties the store).

`clear_session` gains `keep_unsent: Option<KeepUnsentArg>` after `forget_email`, with

```rust
/// `clear_session`'s `keepUnsent: { askId }` (spec 2026-10-10 §5.1, the contract). Accepted and ignored off macOS (W5).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeepUnsentArg {
    ask_id: String,
}
```

and it and calls `clear_session_impl(&state, forget_email.unwrap_or(false), keep_unsent.as_ref().map(|keep| keep.ask_id.as_str()))`. The menu calls `clear_session_impl(&state, false, None)`. Every test call gains `, None`:

```bash
cd $WT/src-tauri && grep -n "clear_session_impl(&" src/lib.rs
```

- [ ] **Step 4: Run, expect PASS.** Filters `sign_out::snapshot` (`ok. 2 passed`), `sign_out_keep_tests` (`ok. 19 passed`: 6 before, 13 new), then `signout_teardown_tests` and `finder_setup_wiring_tests` at their baseline counts.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Remove the gate call: U6 red (the sign-out completes, the queue is empty).
  2. Move the gate below `take_slot_and_stop_engine_for_sign_out(`: U7 red.
  3. Keep only the ops the gate counted (pass `Acknowledge::Asked` built from the gate's read, and copy only its files): U9 red.
  4. Put the keep call inside the `else` arm: U10 red on the order, and U41 red.
  5. Put `purge_macos_hydrate_cache("x")` between the keep call and the purge: U10 red on the deny-list.
  6. Drop the gate's missing-write comparison: U29 (b) red (`unsent_copy_failed:` instead of `unsent_changes:`).
  7. Write the record in the `Err` arm of `clear_session_impl`: U30's source pin red.
  8. Drop `#[cfg(target_os = "macos")]` from the gate's call: U32 red.
  9. Read the store's newest snapshot at the keep step instead of `accepted`: U37 red (the second rename is dropped unasked).
  10. Leave the store as it is after a completed sign-out: U37 red on the last assertion.
  11. Wrap the guard's refusal in `SIGN_OUT_PURGE_FAILED` inside `purge_local_data_for_sign_out` (Task 6, mutation 5): `a_guard_refusal_reaches_the_caller_with_its_code` red on "never wrapped".

- [ ] **Step 6: The task gate** with `<N>` = 8 and `<L>` = `L0 + 58`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/snapshot.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
git commit -m "feat(macos): a sign-out asks before it purges, keeps what waits, and is gated in Rust (1887)" -- src-tauri/src/sign_out/snapshot.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
```

### Task 9: An interrupted sign-out, by a real process death (U42)

**Files:**
- Modify: `src-tauri/src/sign_out/mod.rs` (`crash_point`, test builds only)
- Modify: `src-tauri/src/sign_out/keep.rs` (one crash point after the durable save)
- Modify: `src-tauri/src/lib.rs`, `purge_local_state_files_with` (Task 6): three crash points, and the waiting group removed one path at a time
- Test: `src-tauri/src/lib.rs` `mod sign_out_keep_tests`

**Interfaces:**
- Consumes: `keep_unsent` (Task 5), `purge_local_state_files_with` and `purge_local_data_for_sign_out` (Task 6), `AskSnapshot::of` (Task 8), `kept_record::unfinished_at` (Task 4), `config::CONFIG_DIR_OVERRIDE`.
- Produces: `sign_out::crash_point(point: &str)` (`#[cfg(test)]`): when `BB_TEST_CRASH_AT` equals `point`, it writes `reached:<point>` to the file `BB_TEST_CRASH_REACHED` names and calls `std::process::abort()`. Points: `AfterKeepSave`, `AfterFirstGroup`, `BetweenWaitingCopies`, `BeforeRowClear`.

- [ ] **Step 1: Write the failing test** in `sign_out_keep_tests`:

```rust
    const U42: &str = "sign_out_keep_tests::an_interrupted_sign_out_still_names_its_folder_after_a_restart";

    /// The layout the parent and the child share: `state.db`, staging, the kept root and the config base.
    struct CrashDir {
        dir: PathBuf,
    }

    impl CrashDir {
        fn db(&self) -> state_db::StateDb {
            state_db::StateDb::open(state_paths::state_db_path_from_state_dir(&self.dir)).unwrap()
        }
        fn kept_root(&self) -> PathBuf {
            self.dir.join("kept")
        }
        async fn config(&self) -> PathBuf {
            crate::config::CONFIG_DIR_OVERRIDE.scope(self.dir.join("config"), async { DesktopConfig::path().unwrap() }).await
        }
        fn seed(&self) {
            let db = self.db();
            for op in ["w1", "w2", "w3"] {
                let staged = self.dir.join("staging").join(op);
                std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
                std::fs::write(&staged, op.as_bytes()).unwrap();
                db.hold_lock_for_test()
                    .execute_batch(&format!(
                        "INSERT INTO files (file_id, path) VALUES ('{op}', '{op}.txt');
                         INSERT INTO operation_queue (op_id, kind, file_id, payload_path, write_id, write_origin)
                         VALUES ('{op}', 'upload_version', '{op}', '{p}', '{op}', 'minted');
                         INSERT INTO staged_payloads (path, completed) VALUES ('{p}', 0);",
                        p = staged.display()
                    ))
                    .unwrap();
            }
        }
        /// One sign-out's keep and purge, acknowledging the writes already missing.
        async fn sign_out(&self) -> Result<KeptSet, String> {
            let db = self.db();
            let config = self.config().await;
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&db).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", Vec::new());
            let kept = keep_unsent(KeepInputs {
                db: &db,
                acknowledge: Acknowledge::Asked { others: &snapshot.others, missing: &snapshot.missing },
                root: &self.kept_root(),
                event_name: "event".into(),
                config_path: &config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .map_err(|failure| failure.message())?;
            purge_local_state_files_with(&db, None, &[], None, Some(&kept), &|path| std::fs::remove_file(path))?;
            Ok(kept)
        }
        /// Run the child to `point`; require that it reached it and died by a signal.
        fn crash_at(&self, point: &str) {
            use std::os::unix::process::ExitStatusExt;
            let reached = self.dir.join("reached");
            let _ = std::fs::remove_file(&reached);
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", U42, "--test-threads=1", "--nocapture"])
                .env("BB_TEST_CRASH_CHILD", &self.dir)
                .env("BB_TEST_CRASH_AT", point)
                .env("BB_TEST_CRASH_REACHED", &reached)
                .env("HOME", self.dir.join("home"))
                .status()
                .unwrap();
            assert_eq!(std::fs::read_to_string(&reached).ok().as_deref(), Some(format!("reached:{point}").as_str()), "the child reached {point}");
            assert_eq!(status.signal(), Some(libc::SIGABRT), "the child died there, with no return");
        }
        /// After a "restart": the count from what the child left on disk.
        async fn count(&self) -> (usize, Vec<String>, Option<String>) {
            let config = self.config().await;
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&self.db()).unwrap();
            let unfinished = crate::sign_out::kept_record::unfinished_at(&config).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", unfinished);
            let row = DesktopConfig::load_from(&config).unwrap().kept_unsent_folder;
            (snapshot.counts.missing_files, snapshot.counts.earlier_folders, row)
        }
    }

    /// U42.
    #[tokio::test]
    async fn an_interrupted_sign_out_still_names_its_folder_after_a_restart() {
        if let Ok(dir) = std::env::var("BB_TEST_CRASH_CHILD") {
            let _ = CrashDir { dir: PathBuf::from(dir) }.sign_out().await;
            return; // reached only if the crash point was never hit; the parent then fails on `reached`
        }
        let f1 = |c: &CrashDir| c.kept_root().join("event").to_string_lossy().into_owned();
        for (point, missing) in [("AfterKeepSave", 0), ("AfterFirstGroup", 0), ("BetweenWaitingCopies", 1), ("BeforeRowClear", 3)] {
            let scratch = tempfile::tempdir().unwrap();
            let c = CrashDir { dir: scratch.path().to_path_buf() };
            c.seed();
            c.crash_at(point);
            let (missing_files, earlier, row) = c.count().await;
            assert_eq!(missing_files, missing, "{point}");
            let expected: Vec<String> = if missing > 0 { vec![f1(&c)] } else { Vec::new() };
            assert_eq!(earlier, expected, "{point}");
            assert_eq!(row.as_deref(), Some(f1(&c).as_str()), "{point}");
            assert_eq!(crate::sign_out::kept_record::unfinished_at(&c.config().await).unwrap(), [f1(&c)], "{point}");
            assert_eq!(std::fs::read(Path::new(&f1(&c)).join("w1.txt")).unwrap(), b"w1", "{point}: F1 holds the bytes");
        }
        // (e) From (c)'s leftovers, a second sign-out dies at (c) again: the list holds both, oldest first.
        let scratch = tempfile::tempdir().unwrap();
        let c = CrashDir { dir: scratch.path().to_path_buf() };
        c.seed();
        c.crash_at("BetweenWaitingCopies");
        c.crash_at("BetweenWaitingCopies");
        let f2 = c.kept_root().join("event (2)").to_string_lossy().into_owned();
        let config = c.config().await;
        assert_eq!(crate::sign_out::kept_record::unfinished_at(&config).unwrap(), [f1(&c), f2.clone()]);
        assert_eq!(c.count().await.2.as_deref(), Some(f2.as_str()));
        assert_eq!(std::fs::read(Path::new(&f1(&c)).join("w1.txt")).unwrap(), b"w1", "F1 still holds its files");
        // (f) A sign-out that completes clears the list and never touches 1882's key.
        crate::finder_removal::remember_kept_folder_at(&config, "/macos/kept").unwrap();
        let kept = {
            let db = c.db();
            let waiting = crate::sign_out::waiting::WaitingChanges::read(&db).unwrap();
            let snapshot = AskSnapshot::of(&waiting, "o", Vec::new());
            keep_unsent(KeepInputs {
                db: &db,
                acknowledge: Acknowledge::Asked { others: &snapshot.others, missing: &snapshot.missing },
                root: &c.kept_root(),
                event_name: "event".into(),
                config_path: &config,
                clone: &SystemClone,
                config_fs: &RealConfigFs,
            })
            .unwrap()
        };
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(c.dir.join("config"), purge_local_data_for_sign_out_async(c.dir.clone(), kept))
            .await;
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(!text.contains("kept_unsent_unfinished"), "{text}");
        assert_eq!(DesktopConfig::load_from(&config).unwrap().kept_unsynced_folder.as_deref(), Some("/macos/kept"));
    }

    async fn purge_local_data_for_sign_out_async(dir: PathBuf, kept: KeptSet) {
        purge_local_data_for_sign_out(Ok(dir), None, false, Some(&kept)).unwrap();
    }
```

`purge_local_data_for_sign_out` opens `state_db_from_state_dir(dir)`, the same file `CrashDir::db` opens (`state_paths::state_db_path_from_state_dir`, `@184:state_paths.rs:82-84`).

- [ ] **Step 2: Run it, expect FAIL.** Filter `an_interrupted_sign_out`. Expected: the first point's `reached` assertion fails (no crash point exists, so the child runs to the end).

- [ ] **Step 3: Implement.** In `sign_out/mod.rs`:

```rust
/// Test builds only (spec 2026-10-10 U42): a real process death at `point` when `BB_TEST_CRASH_AT` names it. It
/// writes `reached:<point>` first, so the parent can tell a death here from a death anywhere else.
#[cfg(test)]
pub(crate) fn crash_point(point: &str) {
    if std::env::var("BB_TEST_CRASH_AT").as_deref() == Ok(point) {
        if let Ok(path) = std::env::var("BB_TEST_CRASH_REACHED") {
            let _ = std::fs::write(path, format!("reached:{point}"));
        }
        std::process::abort();
    }
}
```

In `keep_unsent`, right after the durable save succeeded: `#[cfg(test)] super::crash_point("AfterKeepSave");`. In `purge_local_state_files_with`, replace the `for group in [others_first, waiting_last]` loop with:

```rust
    let (removed, skipped) = remove_purge_paths(&others_first, remove)?;
    files_removed += removed;
    files_skipped += skipped;
    #[cfg(test)]
    sign_out::crash_point("AfterFirstGroup");
    for (_index, path) in waiting_last.iter().enumerate() {
        #[cfg(test)]
        if _index == 1 {
            sign_out::crash_point("BetweenWaitingCopies");
        }
        let (removed, skipped) = remove_purge_paths(std::slice::from_ref(path), remove)?;
        files_removed += removed;
        files_skipped += skipped;
    }
    #[cfg(test)]
    sign_out::crash_point("BeforeRowClear");
```

(`_index` is read only in a test build; the underscore keeps a release build free of the unused-variable warning.)

- [ ] **Step 4: Run, expect PASS.** Filter `an_interrupted_sign_out`: `ok. 1 passed`. The child runs print `reached:` files only inside the scratch dirs.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Save the marker only when the sign-out returns `Err` (move the `kept_unsent_unfinished` push out of `record_kept_unsent` into `clear_session_impl`'s `Err` path): every point red on the list.
  2. Move the durable save to just before the second group: `AfterKeepSave` and `AfterFirstGroup` red.
  3. `record_kept_unsent` replaces the list: (e) red on `[F1, F2]`.
  4. Never clear the list: (f) red.

- [ ] **Step 6: The task gate** with `<N>` = 9 and `<L>` = `L0 + 60`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
git commit -m "test(macos): an interrupted sign-out still names its kept folder after a real crash (1887)" -- src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
```

### Task 10: The alert, the row's commands, "Show in Finder", logs, Rust copy

**Files:**
- Modify: `src-tauri/src/sign_out/copy.rs` (every §11 string, the builders, the alert's body)
- Modify: `src-tauri/src/lib.rs`: `surface_sign_out_folders` beside `surface_kept_folder` (`@184:lib.rs:3355-3376`); its two callers, `clear_session` (`:3320`) and the menu (`:14002`); three commands, registered in `generate_handler!` (`:13111`)
- Test: `src-tauri/src/sign_out/copy.rs`, `src-tauri/src/sign_out/keep.rs`, `src-tauri/src/lib.rs` `mod sign_out_keep_tests`

**Interfaces:**
- Consumes: `KeptUnsent` (Task 8), `kept_record::{saved_kept_unsent_at, dismiss_saved_kept_unsent_at}` (Task 4), `finder_removal::{PRESERVED_FILES_TITLE, PRESERVED_FILES_SENTENCE, preserved_files_message, DismissOutcome}`, `save_kept_folder_for_row`, `emit_kept_folder_changed`, `tauri_plugin_opener::OpenerExt::reveal_item_in_dir`.
- Produces:
  - `copy.rs` constants (each mirrored in `src/signOutCopy.ts` by Task 16 under the same name): `P0_TITLE, P1_ONE, P1_MANY, P2_HINT, P3_STOP, A0_TITLE, A0S_TITLE, A1S_LINE, A2_ONE, A2_MANY, A2_COPY_IT, A2_COPY_THEM, A2_COPY_SOME, A2S_ONE, A2S_MANY, A2S_COPY_IT, A2S_COPY_THEM, A2S_COPY_SOME, A3_ONE_OF_ONE, A3_ONE_OF_MANY, A3_MANY, A3E_ONE, A3E_MANY, A4_ONE, A4_MANY, A4S_ONE, A4S_MANY, A5_OFFLINE, A5_SESSION, A5_PAUSED, A5_HELD, A5_ENGINE_STOPPED, A5_TIME_LIMIT, A5_ACCOUNT, ASK_CANCEL, ASK_SIGN_OUT, ASK_SWITCH, K0_TITLE, K1_ONE, K1_MANY, K2_ROW, K2_SHOW, K2_DISMISS, K3_TOAST` and the three E strings (Task 5).
  - Builders: `p1(n) -> Option<String>`, `a2(n, c, switch: bool) -> Option<String>`, `a3(n, k) -> Option<String>`, `a3e(folders: usize) -> Option<&'static str>`, `a4(m, either: bool, switch: bool) -> Option<String>`, `k1(n) -> Option<String>`, `sign_out_alert(ours: Option<(&str, usize)>, macos: Option<&str>) -> Option<(&'static str, String)>`.
  - `lib.rs`: `surface_sign_out_folders(app, &Result<SignedOut, SignOutFailure>)`; commands `kept_unsent_folder() -> Result<Option<String>, String>`, `dismiss_kept_unsent_folder(app, path: String) -> Result<DismissOutcome, String>`, `show_kept_unsent_folder(app, path: String) -> Result<(), String>`; `fn reveal_target<'a>(saved: Option<&str>, path: &'a str) -> Result<&'a str, String>`.

- [ ] **Step 1: Write the failing tests.** In `copy.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// U24.
    #[test]
    fn no_count_string_renders_zero() {
        assert_eq!(p1(0), None);
        assert_eq!(p1(1).as_deref(), Some("Sending 1 change before signing out…"));
        assert_eq!(p1(3).as_deref(), Some("Sending 3 changes before signing out…"));
        assert_eq!(a2(0, 0, false), None);
        assert_eq!(a2(1, 1, false).as_deref(), Some("1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where."));
        assert_eq!(a2(3, 3, false).as_deref(), Some("3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies them to a folder on this Mac and shows you where."));
        assert_eq!(a2(3, 1, false).as_deref(), Some("3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies 1 of them to a folder on this Mac and shows you where."));
        assert_eq!(a2(2, 0, false).as_deref(), Some("2 files you changed in Finder haven’t reached the server."), "never \"copies 0 of them\"");
        assert_eq!(a2(1, 1, true).as_deref(), Some("1 file changed in Finder while that account was signed in hasn’t reached the server. Before switching, Beebeeb copies it to a folder on this Mac and shows you where."));
        assert_eq!(a3(1, 0), None);
        assert_eq!(a3(1, 1).as_deref(), Some(A3_ONE_OF_ONE));
        assert_eq!(a3(4, 1).as_deref(), Some(A3_ONE_OF_MANY));
        assert_eq!(a3(4, 2).as_deref(), Some("Beebeeb’s copies of changes to 2 of these files are missing, so those changes can’t be kept."));
        assert_eq!(a3e(0), None);
        assert_eq!(a3e(1), Some(A3E_ONE));
        assert_eq!(a3e(2), Some(A3E_MANY));
        assert_eq!(a4(0, true, false), None);
        assert_eq!(a4(1, true, false).as_deref(), Some("1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Signing out drops it, and that item stays as it is on the server."));
        assert_eq!(a4(2, false, true).as_deref(), Some("2 other changes (renames, moves, deletions, new folders or restores) haven’t reached the server. Switching drops them, and those items stay as they are on the server."));
        assert_eq!(k1(0), None);
        assert_eq!(k1(1).as_deref(), Some("Beebeeb copied 1 file with changes that hadn’t reached the server to this folder on this Mac:"));
        assert_eq!(k1(2).as_deref(), Some("Beebeeb copied 2 files with changes that hadn’t reached the server to this folder on this Mac:"));
        for text in [P1_MANY, A2_MANY, A3_MANY, A4_MANY, K1_MANY, P0_TITLE, A0_TITLE, K2_ROW, K3_TOAST, E1_UNSENT_CHANGES, E2_UNSENT_COPY_FAILED] {
            assert!(!text.contains('\''), "typographic apostrophes only: {text}");
            assert!(!text.contains("..."), "the one-character ellipsis: {text}");
        }
    }

    /// U23 (lead ruling [1887-two-folders]).
    #[test]
    fn one_alert_names_both_folders_ours_first() {
        use crate::finder_removal::{PRESERVED_FILES_SENTENCE, PRESERVED_FILES_TITLE, preserved_files_message};
        assert_eq!(K0_TITLE, PRESERVED_FILES_TITLE);
        assert_eq!(
            PRESERVED_FILES_SENTENCE,
            "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:",
            "1882's sentence stays verbatim"
        );
        assert_eq!(sign_out_alert(None, None), None);
        assert_eq!(
            sign_out_alert(Some(("/ours", 2)), None),
            Some((K0_TITLE, format!("{}\n\n/ours", k1(2).unwrap())))
        );
        assert_eq!(sign_out_alert(None, Some("/macos")), Some((K0_TITLE, preserved_files_message("/macos"))));
        assert_eq!(
            sign_out_alert(Some(("/ours", 1)), Some("/macos")),
            Some((K0_TITLE, format!("{}\n\n/ours\n\n{PRESERVED_FILES_SENTENCE}\n\n/macos", k1(1).unwrap())))
        );
    }
}
```

In `keep.rs`'s tests (copy `capture_logs` from `@184:ipc_socket.rs:5462-5492` into this module, with its comment):

```rust
    /// U25.
    #[test]
    fn logs_never_carry_a_kept_path_or_name() {
        let f = Fixture::new();
        f.file("secret", "Secret Project/plan for Q4.docx");
        f.write("ws", "secret", "minted", Some(b"plan"));
        let logs = capture_logs(|| {
            f.keep(Acknowledge::Nothing).unwrap();
        });
        assert!(logs.contains("kept unsent changes"), "{logs}");
        for leaked in ["Secret Project", "plan for Q4", ".docx", "Changes not sent", f.dir.path().to_str().unwrap()] {
            assert!(!logs.contains(leaked), "{leaked} in {logs}");
        }
    }
```

In `sign_out_keep_tests`:

```rust
    /// U26.
    #[test]
    fn show_in_finder_reveals_only_the_saved_folder() {
        assert_eq!(reveal_target(Some("/kept/a"), "/kept/a"), Ok("/kept/a"));
        assert!(reveal_target(Some("/kept/a"), "/Users/a/Desktop").is_err());
        assert!(reveal_target(None, "/kept/a").is_err());
    }

    /// U31.
    #[test]
    fn show_in_finder_uses_the_opener_reveal() {
        let production = super::finder_setup_command_tests::production_source();
        let body = super::finder_setup_command_tests::body_between(&production, "fn show_kept_unsent_folder(", "\n}\n");
        assert!(body.contains("reveal_item_in_dir("), "{body}");
        assert!(!body.contains("Command::new(\"open\")") && !body.contains("\"/usr/bin/open\""), "{body}");
        assert!(body.contains("\"kept folder reveal requested\""), "D2 reads this line from stdout");
    }

    /// W9, §9: both sign-out routes surface both folders, through one function.
    #[test]
    fn both_sign_out_routes_surface_both_folders() {
        let production = super::finder_setup_command_tests::production_source();
        let command = super::finder_setup_command_tests::body_between(&production, "async fn clear_session(", "\n}\n");
        assert!(command.contains("surface_sign_out_folders(&app, &result)"), "{command}");
        let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
        assert!(menu.contains("surface_sign_out_folders(&app, &result)"), "{menu}");
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `sign_out::copy`, `logs_never_carry`, `show_in_finder`, `both_sign_out_routes`. Expected: compile errors.

- [ ] **Step 3: Implement.** Append to `copy.rs` (above its tests). Every string is spec §11's, character for character:

```rust
pub const P0_TITLE: &str = "Sending changes first";
pub const P1_ONE: &str = "Sending 1 change before signing out…";
pub const P1_MANY: &str = "Sending {n} changes before signing out…";
pub const P2_HINT: &str = "This takes at most a minute.";
pub const P3_STOP: &str = "Stop waiting";

pub const A0_TITLE: &str = "Sign out with changes not sent?";
pub const A0S_TITLE: &str = "Switch accounts with changes not sent?";
pub const A1S_LINE: &str = "This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.";
pub const A2_ONE: &str = "1 file you changed in Finder hasn’t reached the server.";
pub const A2_MANY: &str = "{n} files you changed in Finder haven’t reached the server.";
pub const A2_COPY_IT: &str = "Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.";
pub const A2_COPY_THEM: &str = "Before signing out, Beebeeb copies them to a folder on this Mac and shows you where.";
pub const A2_COPY_SOME: &str = "Before signing out, Beebeeb copies {c} of them to a folder on this Mac and shows you where.";
pub const A2S_ONE: &str = "1 file changed in Finder while that account was signed in hasn’t reached the server.";
pub const A2S_MANY: &str = "{n} files changed in Finder while that account was signed in haven’t reached the server.";
pub const A2S_COPY_IT: &str = "Before switching, Beebeeb copies it to a folder on this Mac and shows you where.";
pub const A2S_COPY_THEM: &str = "Before switching, Beebeeb copies them to a folder on this Mac and shows you where.";
pub const A2S_COPY_SOME: &str = "Before switching, Beebeeb copies {c} of them to a folder on this Mac and shows you where.";
pub const A3_ONE_OF_ONE: &str = "Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.";
pub const A3_ONE_OF_MANY: &str = "Beebeeb’s copy of a change to 1 of these files is missing, so that change can’t be kept.";
pub const A3_MANY: &str = "Beebeeb’s copies of changes to {k} of these files are missing, so those changes can’t be kept.";
pub const A3E_ONE: &str = "A sign-out that didn’t finish copied changes to this folder, so a change listed as missing may be there:";
pub const A3E_MANY: &str = "Sign-outs that didn’t finish copied changes to these folders, so a change listed as missing may be in one of them:";
pub const A4_ONE: &str = "1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Signing out drops it, and that item stays as it is on the server.";
pub const A4_MANY: &str = "{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Signing out drops them, and those items stay as they are on the server.";
pub const A4S_ONE: &str = "1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server{either}. Switching drops it, and that item stays as it is on the server.";
pub const A4S_MANY: &str = "{m} other changes (renames, moves, deletions, new folders or restores) haven’t reached the server{either}. Switching drops them, and those items stay as they are on the server.";
pub const A5_OFFLINE: &str = "This Mac can’t reach the server right now.";
pub const A5_SESSION: &str = "The server no longer accepts this Mac’s sign-in.";
pub const A5_PAUSED: &str = "Sync is paused.";
pub const A5_HELD: &str = "These are on hold after earlier failures.";
pub const A5_ENGINE_STOPPED: &str = "Sync stopped before everything was sent.";
pub const A5_TIME_LIMIT: &str = "Sending took longer than a minute. Choose Cancel to let it finish, then sign out again.";
/// Q2, option A (spec question 4); the lead rules on the wording.
pub const A5_ACCOUNT: &str = "The server isn’t accepting uploads for this account until it has free space or an active plan.";
pub const ASK_CANCEL: &str = "Cancel";
pub const ASK_SIGN_OUT: &str = "Sign out";
pub const ASK_SWITCH: &str = "Sign out and switch";

pub const K0_TITLE: &str = "Files kept on this Mac";
pub const K1_ONE: &str = "Beebeeb copied 1 file with changes that hadn’t reached the server to this folder on this Mac:";
pub const K1_MANY: &str = "Beebeeb copied {n} files with changes that hadn’t reached the server to this folder on this Mac:";
pub const K2_ROW: &str = "Beebeeb copied changes that had not reached the server to this folder:";
pub const K2_SHOW: &str = "Show in Finder";
pub const K2_DISMISS: &str = "Dismiss";
pub const K3_TOAST: &str = "That folder isn’t on this Mac any more. It may have been moved or deleted.";

fn fill(template: &str, key: &str, value: usize) -> String {
    template.replace(key, &value.to_string())
}

/// P1. Never for 0 (W2): the phase ends first.
pub fn p1(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some(P1_ONE.to_string()),
        n => Some(fill(P1_MANY, "{n}", n)),
    }
}

/// A2 or A2s: the file count, then the copy sentence by c (re-review N1), left out for c = 0.
pub fn a2(n: usize, c: usize, switch: bool) -> Option<String> {
    let (one, many, it, them, some) = if switch {
        (A2S_ONE, A2S_MANY, A2S_COPY_IT, A2S_COPY_THEM, A2S_COPY_SOME)
    } else {
        (A2_ONE, A2_MANY, A2_COPY_IT, A2_COPY_THEM, A2_COPY_SOME)
    };
    let first = match n {
        0 => return None,
        1 => one.to_string(),
        n => fill(many, "{n}", n),
    };
    let second = match c {
        0 => None,
        c if c == n && n == 1 => Some(it.to_string()),
        c if c == n => Some(them.to_string()),
        c => Some(fill(some, "{c}", c)),
    };
    Some(match second {
        Some(second) => format!("{first} {second}"),
        None => first,
    })
}

/// A3, the same in the switch.
pub fn a3(n: usize, k: usize) -> Option<String> {
    match (n, k) {
        (_, 0) => None,
        (1, 1) => Some(A3_ONE_OF_ONE.to_string()),
        (_, 1) => Some(A3_ONE_OF_MANY.to_string()),
        (_, k) => Some(fill(A3_MANY, "{k}", k)),
    }
}

/// A3e's sentence; the frontend puts the paths under it, one per line.
pub fn a3e(folders: usize) -> Option<&'static str> {
    match folders {
        0 => None,
        1 => Some(A3E_ONE),
        _ => Some(A3E_MANY),
    }
}

/// A4 or A4s; " either" only when A2 is shown.
pub fn a4(m: usize, either: bool, switch: bool) -> Option<String> {
    let (one, many) = if switch { (A4S_ONE, A4S_MANY) } else { (A4_ONE, A4_MANY) };
    let text = match m {
        0 => return None,
        1 => one.to_string(),
        m => fill(many, "{m}", m),
    };
    Some(text.replace("{either}", if either { " either" } else { "" }))
}

/// K1, the alert paragraph for our folder.
pub fn k1(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some(K1_ONE.to_string()),
        n => Some(fill(K1_MANY, "{n}", n)),
    }
}

/// §9, lead ruling [1887-two-folders]: one alert per sign-out. Ours first (K1, a blank line, our path), then a blank
/// line and 1882's body verbatim. Only macOS's: 1882's body byte for byte. Neither: no alert.
pub fn sign_out_alert(ours: Option<(&str, usize)>, macos: Option<&str>) -> Option<(&'static str, String)> {
    let ours = ours.and_then(|(path, files)| k1(files).map(|line| format!("{line}\n\n{path}")));
    let macos = macos.map(crate::finder_removal::preserved_files_message);
    let body = match (ours, macos) {
        (None, None) => return None,
        (Some(ours), None) => ours,
        (None, Some(macos)) => macos,
        (Some(ours), Some(macos)) => format!("{ours}\n\n{macos}"),
    };
    Some((K0_TITLE, body))
}
```

In `lib.rs`, beside `surface_kept_folder`:

```rust
/// Spec 2026-10-10 W9 and §9: after a sign-out, ONE alert names our folder, then macOS's. Our folder was recorded by
/// the keep step already; macOS's is saved for its row here, as before (the one route that tells the windows). Neither
/// path is logged.
fn surface_sign_out_folders(app: &tauri::AppHandle, result: &Result<SignedOut, SignOutFailure>) {
    let (macos, ours) = match result {
        Ok(done) => (done.preserved_location.as_deref(), done.kept_unsent.as_ref()),
        Err(failure) => (failure.preserved_location.as_deref(), failure.kept_unsent.as_ref()),
    };
    match macos {
        Some(location) => save_kept_folder_for_row(app, location),
        None if ours.is_some() => emit_kept_folder_changed(app),
        None => {}
    }
    let Some((title, message)) = sign_out::copy::sign_out_alert(ours.map(|kept| (kept.folder.as_str(), kept.files)), macos) else {
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

/// U26: "Show in Finder" reveals only the folder saved for the row.
fn reveal_target<'a>(saved: Option<&str>, path: &'a str) -> Result<&'a str, String> {
    if saved == Some(path) { Ok(path) } else { Err(sign_out::copy::K3_TOAST.to_string()) }
}

/// The row's folder (spec 2026-10-10 §9), or none.
#[tauri::command]
fn kept_unsent_folder() -> Result<Option<String>, String> {
    sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)
}

/// The row's "Dismiss": forgets the folder it showed, never deletes it (§7.3).
#[tauri::command]
fn dismiss_kept_unsent_folder(app: tauri::AppHandle, path: String) -> Result<finder_removal::DismissOutcome, String> {
    let outcome = sign_out::kept_record::dismiss_saved_kept_unsent_at(&DesktopConfig::path()?, &path)?;
    emit_kept_folder_changed(&app);
    Ok(outcome)
}

/// "Show in Finder" (§7.1, review I7): the opener's reveal, never `/usr/bin/open`. On macOS it fails only when the
/// folder is gone, which is what K3 says.
#[tauri::command]
fn show_kept_unsent_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let saved = sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)?;
    let target = reveal_target(saved.as_deref(), &path)?;
    tracing::info!("kept folder reveal requested");
    app.opener().reveal_item_in_dir(target).map_err(|_| sign_out::copy::K3_TOAST.to_string())
}
```

Replace `surface_kept_folder(&app, sign_out_kept_folder(&result));` with `surface_sign_out_folders(&app, &result);` in `clear_session` and in the menu's `DesktopMenuAction::SignOut` arm. Register the three commands in `generate_handler!`. If an 1882 source pin names the old call (`grep -n "sign_out_kept_folder(&result)" src-tauri/src/*.rs`), point it at `surface_sign_out_folders(&app, &result)` and keep what it asserts: both routes surface the folder before they return. `sign_out_kept_folder` then has no caller: delete it.

- [ ] **Step 4: Run, expect PASS.** The four filters: `ok. 2 passed`, `ok. 1 passed`, `ok. 2 passed`, `ok. 1 passed`. Then filter `rb3_f1` and `test_1882`: their baseline counts.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `p1(0)` returns `Some("Sending 0 changes…")`: U24 red.
  2. `a2` copies "{c} of them" for c = 0: U24 red on "never copies 0 of them".
  3. Swap the two paragraphs in `sign_out_alert`: U23 red.
  4. Open 1882's paragraph with "macOS also kept": U23 red.
  5. Drop the blank line between the paragraphs: U23 red.
  6. Log the folder in `log_kept` (`folder = ?kept.folder`): U25 red.
  7. `reveal_target` returns `Ok(path)` always: U26 red.
  8. Reveal with `std::process::Command::new("open").arg("-R")`: U31 red.

- [ ] **Step 6: The task gate** with `<N>` = 10 and `<L>` = `L0 + 66`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/copy.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
git commit -m "feat(macos): one alert names the kept folder and macOS's; the row can show and dismiss it (1887)" -- src-tauri/src/sign_out/copy.rs src-tauri/src/sign_out/keep.rs src-tauri/src/lib.rs
```

### Task 11: The send phase

**Gated on Q2 for its `Account` end only.** Steps marked **Q2-A** are dropped under option B (spec question 4).

**Files:**
- Create: `src-tauri/src/sign_out/send_phase.rs` (pure: the end decision, the progress, the bound)
- Modify: `src-tauri/src/sign_out/mod.rs` (`SignOutState.phase`), `src-tauri/src/sign_out/snapshot.rs` (`AskSnapshot.any_live`), `src-tauri/src/sign_out/waiting.rs` (`readiness`)
- Modify: `src-tauri/src/runner.rs`: `SendPhaseLink`; `EngineRunner` (`@184:runner.rs:617-688`) and `RunnerControls` (`:918-924`) carry it; `run` attaches the link monitor (`:1119`) and gains the third `select!` arm (`:1298-1302`)
- Modify: `src-tauri/src/engine_bridge.rs`: `TransferLoopOutcome` (`@184:engine_bridge.rs:633-646`) gains three signals, set in the failure branch (`:1183-1196`)
- Modify: `src-tauri/src/state_db.rs`: `make_due_for_send_phase`
- Modify: `src-tauri/src/lib.rs`: `sign_out_count`, `sign_out_send_start`, `sign_out_send_stop` (macOS, with same-signature stubs elsewhere that answer an error), the phase driver, registration in `generate_handler!`
- Test: `send_phase.rs`, `state_db.rs`, `engine_bridge.rs`, `runner.rs`, `lib.rs` `sign_out_keep_tests`

**Interfaces:**
- Consumes: `count_ask`, `read_waiting_changes` (Task 8), `SnapshotStore` (Task 8), `send_readiness`, `SendConditions`, `NotSending` (Task 1), `link_health::{LinkMonitor, classify_error}`, `is_curable_refusal`, `OperationFailureClass::Auth`.
- Produces:
  - `send_phase::{SIGN_OUT_SEND_LIMIT, PhaseEnd, PhaseObservation, PhaseStep, SendPhase, phase_end, RunningPhase}`. `PhaseEnd` serializes snake_case: `all_sent`, `held`, `time_limit`, `stopped`, `closed`, `offline`, `session`, `engine_stopped`, `account` (Q2-A).
  - `runner::{SendPhaseLink, AttemptSignals}`; `EngineRunner::send_phase(&self) -> Arc<SendPhaseLink>`; `SendPhaseLink::{arm, disarm, is_armed, wake, link_failed, signals, note}`; `runner::phase_step_due(armed: bool, paused: bool, last_run: Option<Instant>, now: Instant) -> bool`.
  - `TransferLoopOutcome.{link_failed, session_refused, account_refused}: bool`.
  - `StateDb::make_due_for_send_phase(&self, now: i64) -> rusqlite::Result<usize>`.
  - `SignOutState.phase: std::sync::Mutex<Option<RunningPhase>>`; `RunningPhase { ask_id, owner, stop: Arc<AtomicBool>, closed: Arc<AtomicBool> }`.
  - Commands: `sign_out_count(window) -> Result<SignOutCount, String>` (`{ askId, files, copiedFiles, others, missingFiles, earlierFolders, canSend, notSending }`); `sign_out_send_start(window, ask_id: String) -> Result<(), String>`; `sign_out_send_stop() -> Result<(), String>`. Events to the starting window: `sign-out-send-progress` `{ changesLeft }`, `sign-out-send-ended` `{ reason, ask }`.
  - `lib.rs`: `fn start_send_phase(sign_out: &SignOutState, ask_id: &str, owner: &str, link: Option<Arc<runner::SendPhaseLink>>) -> Result<Option<PhaseRun>, String>`, `fn end_send_phase(sign_out: &SignOutState, link: Option<&runner::SendPhaseLink>)`.

- [ ] **Step 1: Write the failing tests.** In `send_phase.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::AttemptSignals;

    fn observing() -> PhaseObservation {
        PhaseObservation {
            elapsed: Duration::from_secs(1),
            changes_left: 2,
            any_live: true,
            engine_running: true,
            session_expired: false,
            link_failed: false,
            attempt: AttemptSignals::default(),
            stop_requested: false,
            window_closed: false,
        }
    }

    /// U16.
    #[test]
    fn the_send_phase_ends_on_each_condition() {
        let base = observing();
        assert_eq!(phase_end(&base), None);
        let cases = [
            (PhaseObservation { changes_left: 0, ..base }, PhaseEnd::AllSent),
            (PhaseObservation { any_live: false, ..base }, PhaseEnd::Held),
            (PhaseObservation { stop_requested: true, ..base }, PhaseEnd::Stopped),
            (PhaseObservation { window_closed: true, ..base }, PhaseEnd::Closed),
            (PhaseObservation { link_failed: true, ..base }, PhaseEnd::Offline),
            (PhaseObservation { attempt: AttemptSignals { link_failed: true, ..Default::default() }, ..base }, PhaseEnd::Offline),
            (PhaseObservation { session_expired: true, ..base }, PhaseEnd::Session),
            (PhaseObservation { attempt: AttemptSignals { session_refused: true, ..Default::default() }, ..base }, PhaseEnd::Session),
            (PhaseObservation { engine_running: false, ..base }, PhaseEnd::EngineStopped),
            // Q2-A
            (PhaseObservation { attempt: AttemptSignals { account_refused: true, ..Default::default() }, ..base }, PhaseEnd::Account),
        ];
        for (observation, end) in cases {
            assert_eq!(phase_end(&observation), Some(end), "{observation:?}");
        }
        assert_eq!(phase_end(&PhaseObservation { elapsed: Duration::from_millis(59_900), ..base }), None, "not at 59.9 s");
        assert_eq!(phase_end(&PhaseObservation { elapsed: Duration::from_secs(60), ..base }), Some(PhaseEnd::TimeLimit), "at exactly 60 s");
        assert_eq!(SIGN_OUT_SEND_LIMIT, Duration::from_secs(60));
        assert_eq!(serde_json::to_value(PhaseEnd::TimeLimit).unwrap(), "time_limit");
    }

    /// §6.4: the progress line changes when the count changes, and never shows 0.
    #[test]
    fn progress_follows_the_count_and_never_reads_zero() {
        let mut phase = SendPhase::default();
        assert_eq!(phase.observe(&observing()), PhaseStep::Progress(2));
        assert_eq!(phase.observe(&observing()), PhaseStep::Continue);
        assert_eq!(phase.observe(&PhaseObservation { changes_left: 1, ..observing() }), PhaseStep::Progress(1));
        assert_eq!(phase.observe(&PhaseObservation { changes_left: 0, ..observing() }), PhaseStep::End(PhaseEnd::AllSent));
    }

    /// U34 (a): the deadline is Rust's: with no webview call at all, the phase ends at 60 s.
    #[test]
    fn the_deadline_needs_no_webview() {
        let mut phase = SendPhase::default();
        let mut ended = None;
        for second in 0..=61u64 {
            if let PhaseStep::End(end) = phase.observe(&PhaseObservation { elapsed: Duration::from_secs(second), ..observing() }) {
                ended = Some((second, end));
                break;
            }
        }
        assert_eq!(ended, Some((60, PhaseEnd::TimeLimit)));
    }
}
```

In `state_db.rs`'s tests:

```rust
    /// U17, the margin; U18.
    #[test]
    fn the_send_phase_makes_live_changes_due_with_a_margin() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('a0', 'upload_version', 'f', 0, 5, 9999, NULL),
                   ('a2', 'rename_file', 'f', 2, 5, 9999, NULL),
                   ('a3', 'upload_version', 'f', 3, 5, 9999, NULL),
                   ('a4', 'upload_version', 'f', 4, 5, 9999, NULL),
                   ('parked', 'upload_version', 'f', 5, 5, 9999, NULL),
                   ('paused', 'upload_version', 'f', 0, 5, 9999, 'auth'),
                   ('hydrate', 'hydrate_file', 'f', 0, 5, 9999, NULL);",
            )
            .unwrap();
        assert_eq!(db.make_due_for_send_phase(100).unwrap(), 2);
        let due: Vec<(String, i64, i64)> = {
            let conn = db.hold_lock_for_test();
            let mut stmt = conn.prepare("SELECT op_id, next_retry_at, attempts FROM operation_queue ORDER BY op_id").unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
        };
        for (op, next, attempts) in due {
            let expected_next = if op == "a0" || op == "a2" { 100 } else { 9999 };
            assert_eq!(next, expected_next, "{op}");
            assert!(attempts <= 5, "attempts never change: {op}");
        }
    }

    #[test]
    fn the_send_phase_never_touches_parked_or_paused_changes() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.hold_lock_for_test()
            .execute_batch(
                "INSERT INTO operation_queue (op_id, kind, file_id, attempts, max_attempts, next_retry_at, paused_reason) VALUES
                   ('parked', 'upload_version', 'f', 25, 25, 9999, NULL), ('paused', 'create_folder', 'f', 0, 5, 9999, 'quota');",
            )
            .unwrap();
        assert_eq!(db.make_due_for_send_phase(100).unwrap(), 0);
        let untouched: i64 = db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue WHERE next_retry_at = 9999 AND attempts IN (0, 25)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(untouched, 2);
    }
```

In `engine_bridge.rs`'s tests (they reach the private `next_attempt_count` and the stub server):

```rust
    /// U17, the arithmetic: a change made due at the phase's start, failing every attempt, is tried at most twice in
    /// 60 s and never parks there, for an other change (`UsesUp`) and for a Finder write's transient failure.
    #[test]
    fn a_phase_never_parks_what_it_made_due() {
        for (max_attempts, start) in [(5_i64, 0_i64), (5, 2), (25, 0), (25, 22)] {
            for count in [AttemptCount::UsesUp, AttemptCount::UpToTheBackoffCap] {
                let mut attempts = start;
                let mut at = 0_i64;
                let mut tries = 0;
                while at <= 60 {
                    attempts = next_attempt_count(attempts, max_attempts, count);
                    tries += 1;
                    at += retry_delay_seconds(attempts);
                }
                assert!(tries <= 2, "{max_attempts}/{start}/{count:?}: {tries} tries");
                assert!(attempts < max_attempts, "{max_attempts}/{start}/{count:?}: parked at {attempts}");
            }
        }
    }

    /// U46 (Q2-A): one attempt that meets a refusal the account cures says so to the send phase.
    #[tokio::test]
    async fn a_refusal_the_account_cures_ends_the_send_phase() {
        let refusals = [quota_exceeded(), object_budget_exceeded(), billing_read_only(), account_lapsed(), plan_required(), trial_cancelled_read_only()];
        for (index, refusal) in refusals.into_iter().enumerate() {
            let key = [190u8 + index as u8; 32];
            let file_id = format!("account-{index}");
            let (dir, sync_root, server, bridge) = rule3_setup(key);
            seed_uploaded_row(&bridge, &server, &file_id);
            refuse(&server, Refused::Init, &file_id, refusal);
            fp_save(&bridge, dir.path(), &file_id, "notes.txt", b"bytes", "1");
            let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
            assert!(outcome.account_refused && !outcome.link_failed && !outcome.session_refused, "{file_id}: {outcome:?}");
            assert!(bridge.db.list_operations_for_file(&file_id).unwrap().iter().all(|op| op.attempts < op.max_attempts), "still live");
        }
        let (dir, sync_root, server, bridge) = rule3_setup([199u8; 32]);
        seed_uploaded_row(&bridge, &server, "plain-413");
        refuse(&server, Refused::Init, "plain-413", ("413 Payload Too Large", json!({ "error": "payload_too_large" })));
        fp_save(&bridge, dir.path(), "plain-413", "notes.txt", b"bytes", "1");
        let outcome = bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        assert!(!outcome.account_refused, "a plain 413 is counted, not an account refusal");
    }
```

In `runner.rs`'s tests:

```rust
    /// U35: the third arm runs the queue step alone, at most once a second, never while paused or disarmed.
    #[test]
    fn the_send_phase_runs_the_queue_step_alone() {
        let start = std::time::Instant::now();
        let mut last = None;
        let mut runs = 0;
        for tenth in 0..=600u64 {
            let now = start + std::time::Duration::from_millis(tenth * 100);
            if phase_step_due(true, false, last, now) {
                runs += 1;
                last = Some(now);
            }
        }
        assert!((60..=61).contains(&runs), "{runs} runs in 60 s");
        assert!(!phase_step_due(true, true, None, start), "not while paused");
        assert!(!phase_step_due(false, false, None, start), "never once disarmed");
        let source = include_str!("runner.rs");
        let arm = &source[source.find("_ = send_phase.wake.notified(), if send_phase.is_armed() =>").expect("the third arm")..];
        let arm = &arm[..arm.find("\n            }\n").unwrap()];
        assert!(arm.contains("process_due_operations("), "{arm}");
        for denied in ["sync_tick_outcome(", "refresh_shared_roots(", "sweep_", "run_known_folder_mirror("] {
            assert!(!arm.contains(denied), "{denied} in the arm");
        }
    }
```

In `lib.rs`'s `sign_out_keep_tests`:

```rust
    /// U34 (f), and (a)'s disarm.
    #[tokio::test]
    async fn the_send_phase_is_owned_by_rust() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            let id = ask(&state, "macos-settings");
            let link = Arc::new(runner::SendPhaseLink::default());
            let stale = start_send_phase(&state.sign_out, "stale", "macos-settings", Some(link.clone()));
            assert!(stale.err().unwrap().starts_with("unsent_changes: "));
            let run = start_send_phase(&state.sign_out, &id, "macos-settings", Some(link.clone())).unwrap();
            assert!(run.is_some() && link.is_armed());
            assert!(start_send_phase(&state.sign_out, &id, "macos-settings", Some(link.clone())).unwrap().is_none(), "a second start starts nothing");
            end_send_phase(&state.sign_out, Some(&link));
            assert!(!link.is_armed() && state.sign_out.phase.lock().unwrap().is_none());
            reset_scratch(&db);
        })
        .await;
    }

    /// §5.1, the contract: `sign_out_count`'s answer.
    #[test]
    fn the_count_answers_the_contract_shape() {
        let snapshot = AskSnapshot::of(&WaitingChanges::default(), "o", Vec::new());
        let json = serde_json::to_value(SignOutCount { counts: snapshot.counts, can_send: false, not_sending: Some(sign_out::waiting::NotSending::Offline) }).unwrap();
        for key in ["askId", "files", "copiedFiles", "others", "missingFiles", "earlierFolders", "canSend", "notSending"] {
            assert!(json.get(key).is_some(), "{key} in {json}");
        }
        assert_eq!(json["notSending"], "offline");
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `sign_out::send_phase`, `the_send_phase_makes`, `never_touches_parked`, `a_phase_never_parks`, `a_refusal_the_account_cures`, `runs_the_queue_step_alone`, `is_owned_by_rust`, `contract_shape`. Expected: compile errors.

- [ ] **Step 3: Implement.**

`send_phase.rs`:

```rust
//! Spec 2026-10-10 §6 (task 1887): the send phase's bound, its end and its progress, as pure state. The driver in
//! `lib.rs` reads the world once a second and hands it here.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::runner::AttemptSignals;

/// §6.3: one constant, from the start of the phase.
pub const SIGN_OUT_SEND_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseEnd {
    AllSent,
    Held,
    TimeLimit,
    Stopped,
    Closed,
    Offline,
    Session,
    EngineStopped,
    /// Q2, option A.
    Account,
}

impl PhaseEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseEnd::AllSent => "all_sent",
            PhaseEnd::Held => "held",
            PhaseEnd::TimeLimit => "time_limit",
            PhaseEnd::Stopped => "stopped",
            PhaseEnd::Closed => "closed",
            PhaseEnd::Offline => "offline",
            PhaseEnd::Session => "session",
            PhaseEnd::EngineStopped => "engine_stopped",
            PhaseEnd::Account => "account",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseObservation {
    pub elapsed: Duration,
    pub changes_left: usize,
    pub any_live: bool,
    pub engine_running: bool,
    pub session_expired: bool,
    pub link_failed: bool,
    pub attempt: AttemptSignals,
    pub stop_requested: bool,
    pub window_closed: bool,
}

/// §6.4, checked at least once a second.
pub fn phase_end(o: &PhaseObservation) -> Option<PhaseEnd> {
    if o.window_closed {
        Some(PhaseEnd::Closed)
    } else if o.stop_requested {
        Some(PhaseEnd::Stopped)
    } else if o.changes_left == 0 {
        Some(PhaseEnd::AllSent)
    } else if !o.engine_running {
        Some(PhaseEnd::EngineStopped)
    } else if o.session_expired || o.attempt.session_refused {
        Some(PhaseEnd::Session)
    } else if o.link_failed || o.attempt.link_failed {
        Some(PhaseEnd::Offline)
    } else if o.attempt.account_refused {
        Some(PhaseEnd::Account)
    } else if !o.any_live {
        Some(PhaseEnd::Held)
    } else if o.elapsed >= SIGN_OUT_SEND_LIMIT {
        Some(PhaseEnd::TimeLimit)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStep {
    Continue,
    Progress(usize),
    End(PhaseEnd),
}

#[derive(Debug, Default)]
pub struct SendPhase {
    last_reported: Option<usize>,
}

impl SendPhase {
    pub fn observe(&mut self, o: &PhaseObservation) -> PhaseStep {
        if let Some(end) = phase_end(o) {
            return PhaseStep::End(end);
        }
        if self.last_reported == Some(o.changes_left) {
            return PhaseStep::Continue;
        }
        self.last_reported = Some(o.changes_left);
        PhaseStep::Progress(o.changes_left)
    }
}

/// The one phase that runs (§5.1 "One flow at a time").
pub struct RunningPhase {
    pub ask_id: String,
    /// The label of the window that started it.
    pub owner: String,
    pub stop: Arc<AtomicBool>,
    pub closed: Arc<AtomicBool>,
}
```

`SignOutState` gains `pub phase: std::sync::Mutex<Option<send_phase::RunningPhase>>`; `mod.rs` gains `pub(crate) mod send_phase;`. `AskSnapshot` gains `pub any_live: bool`, set in `of` from `waiting.any_live()`. `waiting.rs` gains:

```rust
/// [`send_readiness`] on what a snapshot already holds (W2: no second read).
pub fn readiness(conditions: SendConditions, nothing_waits: bool, any_live: bool) -> Option<NotSending> {
    if nothing_waits {
        return None;
    }
    if !conditions.engine_running || !conditions.vault_unlocked {
        Some(NotSending::EngineStopped)
    } else if conditions.session_expired {
        Some(NotSending::Session)
    } else if conditions.link_failed {
        Some(NotSending::Offline)
    } else if conditions.sync_paused {
        Some(NotSending::Paused)
    } else if !any_live {
        Some(NotSending::Held)
    } else {
        None
    }
}
```

and `send_readiness` becomes `readiness(conditions, waiting.is_empty(), waiting.any_live())`.

`runner.rs`:

```rust
/// Spec 2026-10-10 §6.2 (task 1887): the sign-out's send phase, as the runner sees it. Armed only while a phase runs;
/// the runner's third `select!` arm runs the queue step alone then.
#[derive(Default)]
pub struct SendPhaseLink {
    armed: AtomicBool,
    pub(crate) wake: tokio::sync::Notify,
    link: std::sync::OnceLock<Arc<crate::link_health::LinkMonitor>>,
    signals: Mutex<AttemptSignals>,
}

/// What the phase's attempts met, since it was armed (§6.4: `offline`, `session`, Q2-A `account`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttemptSignals {
    pub link_failed: bool,
    pub session_refused: bool,
    pub account_refused: bool,
}

impl SendPhaseLink {
    pub fn arm(&self) {
        *self.signals.lock().unwrap_or_else(|p| p.into_inner()) = AttemptSignals::default();
        self.armed.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::SeqCst)
    }
    pub fn wake(&self) {
        self.wake.notify_one();
    }
    /// §6.1 condition 3: the link monitor has a failure since its last good answer.
    pub fn link_failed(&self) -> bool {
        self.link.get().is_some_and(|link| link.snapshot().failure.is_some())
    }
    pub fn signals(&self) -> AttemptSignals {
        *self.signals.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn attach(&self, link: Arc<crate::link_health::LinkMonitor>) {
        let _ = self.link.set(link);
    }
    pub(crate) fn note(&self, outcome: &crate::engine_bridge::TransferLoopOutcome) {
        let mut signals = self.signals.lock().unwrap_or_else(|p| p.into_inner());
        signals.link_failed |= outcome.link_failed;
        signals.session_refused |= outcome.session_refused;
        signals.account_refused |= outcome.account_refused;
    }
}

/// The third arm's gate (U35): armed, not paused, and at most once a second.
pub(crate) fn phase_step_due(armed: bool, paused: bool, last_run: Option<Instant>, now: Instant) -> bool {
    armed && !paused && last_run.is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
}
```

`EngineRunner` gains `send_phase: Arc<SendPhaseLink>`, made in `spawn` (`let send_phase = Arc::new(SendPhaseLink::default());`), passed in `RunnerControls { …, send_phase: send_phase.clone() }`, kept on `Self`, and handed out by `pub fn send_phase(&self) -> Arc<SendPhaseLink> { self.send_phase.clone() }`. `for_test_with_task` makes a default one. In `run`, after `let link = api.link();`: `send_phase.attach(link.clone());`, and before the loop `let mut last_phase_run: Option<Instant> = None;`. The `select!` gains, after the tick arm:

```rust
            // Spec 2026-10-10 §6.2: while a sign-out's send phase is armed, the queue step alone, at most once a second.
            // Never the remote tick, the sweeps or the shared-root refresh.
            _ = send_phase.wake.notified(), if send_phase.is_armed() => {
                let now = Instant::now();
                if !phase_step_due(send_phase.is_armed(), sync_paused.load(Ordering::Relaxed), last_phase_run, now) {
                    continue;
                }
                last_phase_run = Some(now);
                match bridge.process_due_operations(&sync_root, now_secs()).await {
                    Ok(outcome) => {
                        send_phase.note(&outcome);
                        let (reason, ids) = tick_working_set_signal(&outcome.invalidated_item_ids, &[]);
                        signal_working_set_owing(&mut owed_working_set_signal, &db, reason, &ids);
                    }
                    Err(e) => tracing::warn!(error = %e, "operation queue processing failed during the send phase"),
                }
            }
```

`engine_bridge.rs`: `TransferLoopOutcome` gains

```rust
    /// Spec 2026-10-10 §6.4: an attempt failed at the link level (`offline`), the server refused the session
    /// (`session`), or, Q2-A, the account refused a Finder write it can cure (`account`).
    pub link_failed: bool,
    pub session_refused: bool,
    pub account_refused: bool,
```

(the literal at `@184:engine_bridge.rs:1036` and every literal the compiler names set them `false`), and in the failure branch, after `class` is decided (`:1187-1191`):

```rust
                    outcome.link_failed |= crate::link_health::classify_error(&error).is_some();
                    outcome.session_refused |= class == OperationFailureClass::Auth;
                    outcome.account_refused |= finder_write && is_curable_refusal(&error);
```

`state_db.rs`:

```rust
    /// Spec 2026-10-10 §6.2: every live waiting change with `attempts + 2 < max_attempts` is made due now, once;
    /// `attempts` unchanged. Parked, paused, hydrates, pins and completed uploads are not touched.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn make_due_for_send_phase(&self, now: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue SET next_retry_at = ?1
             WHERE kind NOT IN ('hydrate_file', 'pin_tree')
               AND attempts + 2 < max_attempts
               AND paused_reason IS NULL
               AND op_id NOT IN (SELECT op_id FROM upload_resume WHERE completed_version IS NOT NULL)",
            params![now],
        )
    }
```

`lib.rs` (macOS; each command also gets a `#[cfg(not(target_os = "macos"))]` twin with the same signature whose body is `Err("This works on a Mac only.".into())`, because `generate_handler!` takes no `cfg`):

```rust
/// §5.1, the contract: `sign_out_count`'s answer.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SignOutCount {
    #[serde(flatten)]
    counts: sign_out::snapshot::AskCounts,
    can_send: bool,
    not_sending: Option<sign_out::waiting::NotSending>,
}

/// §6.1 conditions 1-4 for the active account, and the running engine's phase link.
#[cfg(target_os = "macos")]
async fn send_conditions(state: &AppState) -> (sign_out::waiting::SendConditions, Option<Arc<runner::SendPhaseLink>>) {
    let Ok(acct) = state.active_account() else {
        return (sign_out::waiting::SendConditions { engine_running: false, vault_unlocked: false, session_expired: true, link_failed: false, sync_paused: false }, None);
    };
    let link = acct.engine.lock().await.as_ref().map(|engine| engine.send_phase());
    let conditions = sign_out::waiting::SendConditions {
        engine_running: link.is_some(),
        vault_unlocked: acct.session.lock().map(|session| session.is_some()).unwrap_or(false),
        session_expired: acct.auth_health.is_expired(),
        link_failed: link.as_ref().is_some_and(|link| link.link_failed()),
        sync_paused: acct.sync_paused.load(Ordering::Relaxed),
    };
    (conditions, link)
}

/// §5.1 step 1: one read, a snapshot owned by the calling window, and one `info` line with counts only (review I8).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn sign_out_count(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<SignOutCount, String> {
    let snapshot = count_ask(window.label())?;
    let (conditions, _) = send_conditions(&state).await;
    let not_sending = sign_out::waiting::readiness(conditions, snapshot.is_empty(), snapshot.any_live);
    let can_send = !snapshot.is_empty() && not_sending.is_none();
    tracing::info!(
        files = snapshot.counts.files,
        copied_files = snapshot.counts.copied_files,
        others = snapshot.counts.others,
        missing_files = snapshot.counts.missing_files,
        can_send,
        not_sending = not_sending.map(|reason| format!("{reason:?}")).unwrap_or_default(),
        earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
        "sign-out flow counted"
    );
    let counts = snapshot.counts.clone();
    state.sign_out.snapshots.record(snapshot);
    Ok(SignOutCount { counts, can_send, not_sending })
}

/// What the driver needs of a started phase.
#[cfg(target_os = "macos")]
struct PhaseRun {
    owner: String,
    link: Option<Arc<runner::SendPhaseLink>>,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    started: std::time::Instant,
    changes_at_start: usize,
}

/// §6.2: a stale ask refuses; a running phase starts nothing; otherwise make live changes due, arm, and register.
#[cfg(target_os = "macos")]
fn start_send_phase(
    sign_out: &sign_out::SignOutState,
    ask_id: &str,
    owner: &str,
    link: Option<Arc<runner::SendPhaseLink>>,
) -> Result<Option<PhaseRun>, String> {
    if sign_out.snapshots.newest_ask_id().as_deref() != Some(ask_id) {
        return Err(sign_out::copy::coded(sign_out::copy::UNSENT_CHANGES_CODE, sign_out::copy::E1_UNSENT_CHANGES));
    }
    let mut phase = sign_out.phase.lock().unwrap_or_else(|p| p.into_inner());
    if phase.is_some() {
        return Ok(None);
    }
    let changes_at_start = read_waiting_changes().map(|waiting| waiting.changes()).unwrap_or(0);
    if let Ok(Some(db)) = state_paths::beebeeb_state_dir().and_then(|dir| state_db_from_state_dir(&dir)) {
        let _ = db.make_due_for_send_phase(now_unix_seconds());
    }
    if let Some(link) = &link {
        link.arm();
    }
    let (stop, closed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    *phase = Some(sign_out::send_phase::RunningPhase { ask_id: ask_id.to_string(), owner: owner.to_string(), stop: stop.clone(), closed: closed.clone() });
    Ok(Some(PhaseRun { owner: owner.to_string(), link, stop, closed, started: std::time::Instant::now(), changes_at_start }))
}

/// The phase is over, for any reason: disarm, so the runner is back on its tick within a second.
#[cfg(target_os = "macos")]
fn end_send_phase(sign_out: &sign_out::SignOutState, link: Option<&runner::SendPhaseLink>) {
    if let Some(link) = link {
        link.disarm();
    }
    *sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn sign_out_send_start(app: tauri::AppHandle, window: tauri::WebviewWindow, state: State<'_, AppState>, ask_id: String) -> Result<(), String> {
    let (_, link) = send_conditions(&state).await;
    if let Some(run) = start_send_phase(&state.sign_out, &ask_id, window.label(), link)? {
        tauri::async_runtime::spawn(drive_send_phase(app, run));
    }
    Ok(())
}

/// "Stop waiting" (and Esc): the phase ends `stopped`. Nothing happens when no phase runs.
#[cfg(target_os = "macos")]
#[tauri::command]
fn sign_out_send_stop(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(phase) = state.sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        phase.stop.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// The driver: once a second, wake the runner's queue step, read the world, and end or report progress.
#[cfg(target_os = "macos")]
async fn drive_send_phase(app: tauri::AppHandle, run: PhaseRun) {
    let state = app.state::<AppState>();
    let mut phase = sign_out::send_phase::SendPhase::default();
    let mut changes_left = run.changes_at_start;
    let end = loop {
        if let Some(link) = &run.link {
            link.wake();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Ok(waiting) = read_waiting_changes() else {
            break sign_out::send_phase::PhaseEnd::Stopped;
        };
        changes_left = waiting.changes();
        let (conditions, _) = send_conditions(&state).await;
        let observation = sign_out::send_phase::PhaseObservation {
            elapsed: run.started.elapsed(),
            changes_left,
            any_live: waiting.any_live(),
            engine_running: run.link.is_some() && conditions.engine_running,
            session_expired: conditions.session_expired,
            link_failed: conditions.link_failed,
            attempt: run.link.as_ref().map(|link| link.signals()).unwrap_or_default(),
            stop_requested: run.stop.load(Ordering::SeqCst),
            window_closed: run.closed.load(Ordering::SeqCst),
        };
        match phase.observe(&observation) {
            sign_out::send_phase::PhaseStep::Continue => {}
            sign_out::send_phase::PhaseStep::Progress(left) => {
                let _ = app.emit_to(run.owner.as_str(), "sign-out-send-progress", serde_json::json!({ "changesLeft": left }));
            }
            sign_out::send_phase::PhaseStep::End(end) => break end,
        }
    };
    end_send_phase(&state.sign_out, run.link.as_deref());
    use sign_out::send_phase::PhaseEnd;
    let ask = match end {
        PhaseEnd::AllSent | PhaseEnd::Closed => None,
        _ => count_ask(&run.owner).ok().map(|snapshot| {
            let counts = snapshot.counts.clone();
            state.sign_out.snapshots.record(snapshot);
            counts
        }),
    };
    tracing::info!(
        reason = end.as_str(),
        changes_at_start = run.changes_at_start,
        changes_left,
        elapsed_ms = run.started.elapsed().as_millis() as u64,
        "sign-out send phase ended"
    );
    let _ = app.emit_to(run.owner.as_str(), "sign-out-send-ended", serde_json::json!({ "reason": end, "ask": ask }));
}
```

(`now_unix_seconds()` is `lib.rs`'s unix-seconds helper, `@184:lib.rs:3756`.) Register `sign_out_count`, `sign_out_send_start`, `sign_out_send_stop` in `generate_handler!`.

- [ ] **Step 4: Run, expect PASS.** The eight filters: `ok. 3`, `ok. 1`, `ok. 1`, `ok. 1`, `ok. 1`, `ok. 1`, `ok. 1`, `ok. 1` passed.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `SIGN_OUT_SEND_LIMIT = 59 s`, then `61 s`: U16 red each time.
  2. Drop the `any_live` check: U16 red on `Held`.
  3. Read only the monitor for `offline` (drop `o.attempt.link_failed`): U16 red on the attempt-only row.
  4. Make due regardless of `attempts` (drop `attempts + 2 < max_attempts`): U17 red on `a3`/`a4`; `a_phase_never_parks…` stays green (it tests the arithmetic), which is why the SQL test exists.
  5. Reset `attempts` in the update (`attempts = 0`): U18 red.
  6. Call `sync_tick_outcome(` in the arm: U35 red.
  7. Drop the once-a-second limit (`phase_step_due` returns `armed && !paused`): U35 red (601 runs).
  8. Q2-A: drop `outcome.account_refused |= …`: U46 red.
  9. Let a second start register a second phase: U34 (f) red.
  10. Leave the link armed in `end_send_phase`: U34 (a) red.

- [ ] **Step 6: The task gate** with `<N>` = 11 and `<L>` = `L0 + 76`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/send_phase.rs src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/snapshot.rs src-tauri/src/sign_out/waiting.rs src-tauri/src/runner.rs src-tauri/src/engine_bridge.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
git commit -m "feat(macos): send waiting changes first, for at most a minute, before a sign-out asks (1887)" -- src-tauri/src/sign_out/send_phase.rs src-tauri/src/sign_out/mod.rs src-tauri/src/sign_out/snapshot.rs src-tauri/src/sign_out/waiting.rs src-tauri/src/runner.rs src-tauri/src/engine_bridge.rs src-tauri/src/state_db.rs src-tauri/src/lib.rs
```

### Task 12: A close ends the flow; a late result is shown

**Files:**
- Create: `src-tauri/src/sign_out/flow_window.rs` (pure: the late-result record)
- Modify: `src-tauri/src/sign_out/mod.rs` (`SignOutState.late_results`)
- Modify: `src-tauri/src/lib.rs`: `flow_window_closed` and `sign_out_flow_window_closed`; the global `.on_window_event` (`@184:lib.rs:13440-13458`); a new `.on_page_load` on the builder; `clear_session` (window parameter, begin and complete); `sign_out_result_ack`; `show_late_result`; registration
- Test: `src-tauri/src/sign_out/flow_window.rs`, `src-tauri/src/lib.rs` `mod sign_out_keep_tests`

**Interfaces:**
- Consumes: `SignOutState.{snapshots, phase}` (Tasks 8, 11), `RunningPhase` (Task 11), `surfaces::policy::{close_policy, Platform}`, `surfaces::registry::ClosePolicy`, the menu's dialog calls (`@184:lib.rs:14006-14036`).
- Produces:
  - `flow_window::{Shown, LateResults}`; `Shown::{Failure(String), Warning(String)}`; `LateResults::{begin(label), complete(label, Option<Shown>) -> Option<Shown>, window_closed(label) -> Option<Shown>, acknowledge(label) -> bool}`. A `Some` return means "show this now, in the native dialog".
  - `lib.rs`: `fn late_result_of(&Result<SignedOut, SignOutFailure>) -> Option<Shown>`; `struct FlowClosed { phase_closed: bool, snapshot_dropped: bool, show: Option<Shown> }`; `fn flow_window_closed(sign_out: &SignOutState, label: &str) -> FlowClosed`; `fn sign_out_flow_window_closed(app: &tauri::AppHandle, label: &str)`; command `sign_out_result_ack(window) -> Result<(), String>`; event `sign-out-flow-closed` (no payload) to the closed window.
  - `clear_session(app, window: tauri::WebviewWindow, state, forget_email, keep_unsent)`.

- [ ] **Step 1: Write the failing tests.** In `flow_window.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> Option<Shown> {
        Some(Shown::Failure("Sign-out paused: purge failed".into()))
    }

    /// U44 (a)-(j).
    #[test]
    fn a_result_that_returned_before_the_close_is_still_shown() {
        let s = "macos-settings";
        // (a) the exact race: returned, then closed, no acknowledgement → one dialog, record gone.
        let late = LateResults::default();
        late.begin(s);
        assert_eq!(late.complete(s, failure()), None, "pending: the window may still show it");
        assert_eq!(late.window_closed(s), failure());
        assert_eq!(late.window_closed(s), None, "(i) a second close (Destroyed) shows nothing");
        // (b) a warning.
        let warning = Some(Shown::Warning("Beebeeb couldn’t confirm Finder was removed.".into()));
        late.begin(s);
        late.complete(s, warning.clone());
        assert_eq!(late.window_closed(s), warning);
        // (c) an acknowledgement after the dialog: nothing, no panic, nothing removed.
        assert!(!late.acknowledge(s));
        // (d) acknowledged first, then the close: no dialog.
        late.begin(s);
        late.complete(s, failure());
        assert!(late.acknowledge(s));
        assert_eq!(late.window_closed(s), None);
        // (e) nothing to show: no record.
        late.begin(s);
        assert_eq!(late.complete(s, None), None);
        assert_eq!(late.window_closed(s), None);
        // (f) closed while running, then complete: shown at completion.
        late.begin(s);
        assert_eq!(late.window_closed(s), None);
        assert_eq!(late.complete(s, failure()), failure());
        // (g) another window's close leaves the record alone.
        late.begin(s);
        late.complete(s, failure());
        assert_eq!(late.window_closed("settings"), None);
        assert_eq!(late.window_closed("onboarding"), None);
        assert_eq!(late.window_closed(s), failure());
        // (j) a second begin from the same window replaces its record.
        late.begin(s);
        late.complete(s, failure());
        late.begin(s);
        assert_eq!(late.window_closed(s), None, "running again: marked, nothing shown yet");
        assert_eq!(late.complete(s, None), None);
    }

    /// U44 (k): the hook and the acknowledgement raced, 200 rounds: one dialog or none, never two.
    #[test]
    fn the_close_and_the_acknowledgement_never_both_show() {
        for round in 0..200 {
            let late = std::sync::Arc::new(LateResults::default());
            late.begin("w");
            late.complete("w", failure());
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let (hook_late, hook_barrier) = (late.clone(), barrier.clone());
            let hook = std::thread::spawn(move || {
                hook_barrier.wait();
                hook_late.window_closed("w")
            });
            barrier.wait();
            let removed = late.acknowledge("w");
            let shown = hook.join().unwrap();
            assert_eq!(shown.is_some(), !removed, "round {round}: shown {shown:?}, acknowledged {removed}");
        }
    }
}
```

In `sign_out_keep_tests`:

```rust
    fn phase_owned_by(state: &AppState, owner: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        *state.sign_out.phase.lock().unwrap() = Some(crate::sign_out::send_phase::RunningPhase {
            ask_id: "a".into(),
            owner: owner.into(),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            closed: closed.clone(),
        });
        closed
    }

    /// U34 (b)-(e), (g), (h).
    #[tokio::test]
    async fn the_send_phase_is_owned_by_rust_and_a_close_ends_the_flow() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            let state = AppState::default();
            // (b) a close during that window's phase: ended `closed`, its snapshot dropped; a later Sign out refuses.
            let id = ask(&state, "macos-settings");
            let closed = phase_owned_by(&state, "macos-settings");
            let outcome = flow_window_closed(&state.sign_out, "macos-settings");
            assert!(outcome.phase_closed && closed.load(Ordering::SeqCst) && outcome.snapshot_dropped);
            let refused = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
            assert!(refused.message.starts_with("unsent_changes: "));
            // (c) at the ask, no phase: the snapshot is dropped.
            *state.sign_out.phase.lock().unwrap() = None;
            ask(&state, "macos-settings");
            assert!(flow_window_closed(&state.sign_out, "macos-settings").snapshot_dropped);
            // (d) another window's close leaves a phase and a snapshot of `macos-settings` alone.
            ask(&state, "macos-settings");
            let closed = phase_owned_by(&state, "macos-settings");
            for other in ["settings", "onboarding"] {
                let outcome = flow_window_closed(&state.sign_out, other);
                assert!(!outcome.phase_closed && !outcome.snapshot_dropped, "{other}");
            }
            assert!(!closed.load(Ordering::SeqCst));
            *state.sign_out.phase.lock().unwrap() = None;
            // (g) a close of `onboarding` drops the snapshot `onboarding` owns.
            ask(&state, "onboarding");
            assert!(flow_window_closed(&state.sign_out, "onboarding").snapshot_dropped);
            // (h) running when its window closes, then failing: the dialog, once.
            state.sign_out.late_results.begin("macos-settings");
            assert_eq!(flow_window_closed(&state.sign_out, "macos-settings").show, None);
            assert!(state.sign_out.late_results.complete("macos-settings", Some(crate::sign_out::flow_window::Shown::Failure("x".into()))).is_some());
            reset_scratch(&db);
        })
        .await;
    }

    /// U34's pins: the hook is called on every close, destroy and reload, with the window's own label; and its premise.
    #[test]
    fn every_close_destroy_and_reload_reaches_the_hook() {
        let production = super::finder_setup_command_tests::production_source();
        let handler = &production[production.find(".on_window_event(|window, event| {").expect("the global handler")..];
        let handler = &handler[..handler.find("\n        })\n").unwrap()];
        for arm in ["if let tauri::WindowEvent::CloseRequested { .. } = event {", "if let tauri::WindowEvent::Destroyed = event {"] {
            let at = handler.find(arm).unwrap_or_else(|| panic!("{arm}"));
            assert!(handler[at..].contains("sign_out_flow_window_closed(window.app_handle(), window.label())"), "{arm}");
        }
        let page_load = &production[production.find(".on_page_load(").expect("an on_page_load registration")..];
        let page_load = &page_load[..page_load.find("\n        })\n").unwrap()];
        assert!(page_load.contains("PageLoadEvent::Started"), "{page_load}");
        assert!(page_load.contains("sign_out_flow_window_closed(webview.app_handle(), webview.label())"), "{page_load}");
        assert_eq!(
            surfaces::policy::close_policy(surfaces::policy::Platform::Macos, "macos-settings"),
            surfaces::registry::ClosePolicy::Hide,
            "the premise: a close never destroys the Settings window"
        );
    }

    /// U32, extended (PR #121 reviews 1, 2): the late result is macOS only, defined and registered.
    #[test]
    fn the_late_result_is_macos_only() {
        let production = super::finder_setup_command_tests::production_source();
        for definition in ["fn sign_out_result_ack(", "fn sign_out_flow_window_closed(", "fn sign_out_count(", "fn sign_out_send_start("] {
            let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
            let before = &production[..at];
            assert!(before[before.rfind("#[cfg(").unwrap()..].starts_with("#[cfg(target_os = \"macos\")]"), "{definition}");
        }
        let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
        assert!(handler.contains("sign_out_result_ack"), "registered");
        let command = super::finder_setup_command_tests::body_between(&production, "async fn clear_session(", "\n}\n");
        for call in ["late_results.begin(", "late_results.complete("] {
            let at = command.find(call).unwrap_or_else(|| panic!("{call}"));
            let before = &command[..at];
            assert!(before[before.rfind("#[cfg(").unwrap()..].starts_with("#[cfg(target_os = \"macos\")]"), "{call}");
        }
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `sign_out::flow_window`, `a_close_ends_the_flow`, `every_close_destroy`, `the_late_result_is_macos_only`. Expected: compile errors.

- [ ] **Step 3: Implement.** `flow_window.rs`:

```rust
//! Spec 2026-10-10 §5.1 "A late result" (task 1887): one record per window that called `clear_session`, from the call's
//! start until its result is acknowledged or shown. Taking the record out is the one atomic step, so a result is shown
//! at most once, and, if the webview never displays it, at least once.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// What the native dialog shows: a failure in "Sign-out paused", a completed sign-out's warning in "Signed out".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Failure(String),
    Warning(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Record {
    Running { closed: bool },
    Pending(Shown),
}

#[derive(Default)]
pub struct LateResults {
    records: Mutex<HashMap<String, Record>>,
}

impl LateResults {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Record>> {
        self.records.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A `clear_session` from `label` started; it replaces that window's record.
    pub fn begin(&self, label: &str) {
        self.lock().insert(label.to_string(), Record::Running { closed: false });
    }

    /// The call returned. `Some`: show it now (its window closed while it ran).
    pub fn complete(&self, label: &str, result: Option<Shown>) -> Option<Shown> {
        let mut records = self.lock();
        match (records.remove(label), result) {
            (Some(Record::Running { closed: true }), result) => result,
            (Some(Record::Running { closed: false }), Some(shown)) => {
                records.insert(label.to_string(), Record::Pending(shown));
                None
            }
            _ => None,
        }
    }

    /// `label` closed, was destroyed or reloads. `Some`: show it now.
    pub fn window_closed(&self, label: &str) -> Option<Shown> {
        let mut records = self.lock();
        match records.get_mut(label) {
            Some(Record::Running { closed }) => {
                *closed = true;
                None
            }
            Some(Record::Pending(_)) => match records.remove(label) {
                Some(Record::Pending(shown)) => Some(shown),
                _ => None,
            },
            None => None,
        }
    }

    /// The webview displayed the result. Whether it removed one.
    pub fn acknowledge(&self, label: &str) -> bool {
        let mut records = self.lock();
        if matches!(records.get(label), Some(Record::Pending(_))) {
            records.remove(label);
            return true;
        }
        false
    }
}
```

`SignOutState` gains `pub late_results: flow_window::LateResults`; `mod.rs` gains `pub(crate) mod flow_window;`. In `lib.rs`:

```rust
/// §5.1 "A late result": what needs showing. Not `unsent_changes` (nothing changed, and a hidden window cannot show
/// the ask), and not a completed sign-out without a warning.
fn late_result_of(result: &Result<SignedOut, SignOutFailure>) -> Option<sign_out::flow_window::Shown> {
    use sign_out::flow_window::Shown;
    match result {
        Err(failure) if failure.message.starts_with(&format!("{}: ", sign_out::copy::UNSENT_CHANGES_CODE)) => None,
        Err(failure) => Some(Shown::Failure(failure.message.clone())),
        Ok(SignedOut { warning: Some(warning), .. }) => Some(Shown::Warning(warning.sentence().to_string())),
        Ok(_) => None,
    }
}

/// The menu's own dialogs (`@184:lib.rs:14006-14036`), callback form, so a window-event handler can raise one.
#[cfg(target_os = "macos")]
fn show_late_result(app: &tauri::AppHandle, shown: sign_out::flow_window::Shown) {
    use sign_out::flow_window::Shown;
    let (title, message, kind) = match shown {
        Shown::Failure(message) => ("Sign-out paused", message, tauri_plugin_dialog::MessageDialogKind::Error),
        Shown::Warning(sentence) => ("Signed out", sentence, tauri_plugin_dialog::MessageDialogKind::Info),
    };
    app.dialog().message(message).title(title).kind(kind).show(|_| {});
}

/// What a close of `label` did (U34): the four steps of §5.1's hook, on the flow state alone.
struct FlowClosed {
    phase_closed: bool,
    snapshot_dropped: bool,
    show: Option<sign_out::flow_window::Shown>,
}

fn flow_window_closed(sign_out: &sign_out::SignOutState, label: &str) -> FlowClosed {
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

/// §5.1 "A close ends the flow": every `CloseRequested` and `Destroyed`, and every page load `Started`, of any window.
#[cfg(target_os = "macos")]
fn sign_out_flow_window_closed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let closed = flow_window_closed(&state.sign_out, label);
    let _ = app.emit_to(label, "sign-out-flow-closed", ());
    if let Some(shown) = closed.show {
        show_late_result(app, shown);
    }
}

/// The webview displayed its `clear_session`'s outcome (F12): remove that window's pending result.
#[cfg(target_os = "macos")]
#[tauri::command]
fn sign_out_result_ack(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    state.sign_out.late_results.acknowledge(window.label());
    Ok(())
}
```

(`window.app_handle()` and `webview.app_handle()` are `&AppHandle<Wry>`, which is `&tauri::AppHandle`.) `sign_out_result_ack` gets a non-macOS twin with the same signature returning `Ok(())`, and is registered in `generate_handler!`.

In the global `.on_window_event` handler, before the close-policy block:

```rust
            // Spec 2026-10-10 §5.1: a close (on macOS a hide), a destroy or a reload ends that window's sign-out flow.
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::Destroyed = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
```

On the builder, before `.on_window_event(`:

```rust
        .on_page_load(|webview, payload| {
            #[cfg(target_os = "macos")]
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                sign_out_flow_window_closed(webview.app_handle(), webview.label());
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (webview, payload);
        })
```

`clear_session` gains `window: tauri::WebviewWindow` after `app`, and around its call:

```rust
    #[cfg(target_os = "macos")]
    state.sign_out.late_results.begin(window.label());
    let result = clear_session_impl(&state, forget_email.unwrap_or(false), keep_unsent.as_ref().map(|keep| keep.ask_id.as_str())).await;
    surface_sign_out_folders(&app, &result);
    #[cfg(target_os = "macos")]
    if let Some(shown) = state.sign_out.late_results.complete(window.label(), late_result_of(&result)) {
        show_late_result(&app, shown);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &window;
```

- [ ] **Step 4: Run, expect PASS.** The four filters: `ok. 2`, `ok. 1`, `ok. 1`, `ok. 1` passed.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `complete` drops the record for a failure (no `Pending`): U44 (a) red.
  2. `window_closed` only marks and never shows a `Pending`: U44 (a) red.
  3. `window_closed` shows but leaves the record: U44 (i) red (a second dialog).
  4. `acknowledge` and `window_closed` each read under the lock, then remove in a second lock: the race test red in some round.
  5. Listen for `Destroyed` only: the pin red.
  6. Pass the constant `"macos-settings"` as the label: the pin red.
  7. `flow_window_closed` drops the snapshot whatever its owner: U34 (d) red.
  8. End the phase `stopped` instead of `closed` (store into `phase.stop`): U34 (b) red.
  9. Drop the `cfg` from `sign_out_result_ack`: the macOS-only pin red.

- [ ] **Step 6: The task gate** with `<N>` = 12 and `<L>` = `L0 + 81`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/flow_window.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
git commit -m "feat(macos): closing Settings ends the sign-out flow, and a late result is shown once (1887)" -- src-tauri/src/sign_out/flow_window.rs src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs
```

### Task 13: Entry points: the flow request, the menu, the switch's snapshot

**Files:**
- Modify: `src-tauri/src/sign_out/mod.rs` (`SignOutState.flow_request`)
- Modify: `src-tauri/src/lib.rs`: `request_sign_out_flow`, `consume_sign_out_flow_request`, `open_sign_out_flow`; the menu's `SignOut` arm (`@184:lib.rs:13994-14040`); `SignInSettlement::AccountMismatch` (`:5183-5185`) and its settlement (`:5240`); `AccountMismatchDto` and `LoginOutcome::account_mismatch` (`:1426-1470`); its two callers (`:1690-1694`, `:1888-1894`); every test call of `LoginOutcome::account_mismatch(`
- Modify: `src-tauri/src/browser_login.rs:495-502` (the event carries the snapshot)
- Test: `src-tauri/src/lib.rs` `mod sign_out_keep_tests`

**Interfaces:**
- Consumes: `count_ask` (Task 8), `SnapshotStore::record` (Task 8), `show_macos_settings_window` (`@184:lib.rs:13029`), `AskCounts` (Task 8).
- Produces:
  - `SignOutState.flow_request: std::sync::atomic::AtomicBool`.
  - Commands `request_sign_out_flow(app) -> Result<(), String>` (the compact page, entry point 2) and `consume_sign_out_flow_request(state) -> bool` (Settings, F9); event `menu:sign-out-flow` to `macos-settings`.
  - `fn open_sign_out_flow(app: &tauri::AppHandle)`; `fn take_sign_out_flow_request(sign_out: &SignOutState) -> bool`.
  - `fn switch_snapshot(state: &AppState) -> Result<Option<AskCounts>, String>` (macOS: `count_ask("onboarding")`, recorded as the newest; elsewhere `Ok(None)`).
  - `AccountMismatchDto { pending_changes: u64, #[serde(skip_serializing_if = "Option::is_none")] unsent: Option<AskCounts> }`; `LoginOutcome::account_mismatch(pending_changes: u64, unsent: Option<AskCounts>)`; `SignInSettlement::AccountMismatch { pending_changes, unsent }`. The browser sign-in's `account_mismatch` event payload adds `"unsent"` only when it is `Some`.

- [ ] **Step 1: Write the failing tests** in `sign_out_keep_tests`:

```rust
    /// U19 (macOS half; the frontend half is F8, the Linux case is F8's last line).
    #[tokio::test]
    async fn the_switch_carries_a_snapshot_on_macos_only() {
        with_the_shared_state_dir(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            queue_rename(&db, "r1");
            let state = AppState::default();
            let unsent = switch_snapshot(&state).unwrap().expect("a snapshot on macOS");
            assert_eq!((unsent.files, unsent.others), (1, 1));
            assert_eq!(state.sign_out.snapshots.newest_ask_id().as_deref(), Some(unsent.ask_id.as_str()), "recorded as the newest");
            assert!(state.sign_out.phase.lock().unwrap().is_none(), "no send is started");
            let json = serde_json::to_value(LoginOutcome::account_mismatch(2, Some(unsent))).unwrap();
            assert_eq!(json["account_mismatch"]["pending_changes"], 2);
            assert!(json["account_mismatch"]["unsent"]["askId"].is_string());
            assert!(state.sign_out.snapshots.drop_owned_by("onboarding"), "owned by the window the switch step runs in");
            let without = serde_json::to_value(LoginOutcome::account_mismatch(2, None)).unwrap();
            assert_eq!(without["account_mismatch"], serde_json::json!({ "pending_changes": 2 }), "elsewhere: unchanged");
            reset_scratch(&db);
        })
        .await;
    }

    /// U21: the menu never purges a waiting write and never shows the error dialog for it.
    #[test]
    fn the_menu_never_purges_a_waiting_write() {
        let state = AppState::default();
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        assert!(take_sign_out_flow_request(&state.sign_out), "one request");
        assert!(!take_sign_out_flow_request(&state.sign_out), "not two");
        let production = super::finder_setup_command_tests::production_source();
        let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
        let flow = menu.find("Err(error) if error.message.starts_with(\"unsent_changes: \") => open_sign_out_flow(&app),").expect("the flow arm");
        let paused = menu.find(".title(\"Sign-out paused\")").unwrap();
        assert!(flow < paused, "the flow arm comes before the error dialog's arm");
        assert!(menu.contains("clear_session_impl(&state, false, None)"), "the menu never keeps on its own");
    }

    /// F9's Rust half: the compact page's request survives a Settings window that has not loaded yet.
    #[test]
    fn a_flow_request_waits_for_settings_to_consume_it() {
        let state = AppState::default();
        assert!(!take_sign_out_flow_request(&state.sign_out));
        state.sign_out.flow_request.store(true, Ordering::SeqCst);
        assert!(take_sign_out_flow_request(&state.sign_out));
    }
```

- [ ] **Step 2: Run them, expect FAIL.** Filters `switch_carries_a_snapshot`, `menu_never_purges`, `flow_request_waits`. Expected: compile errors.

- [ ] **Step 3: Implement.** `SignOutState` gains `pub flow_request: std::sync::atomic::AtomicBool`. In `lib.rs`:

```rust
/// §5.1 "A flow request survives a window that has not loaded yet": one pending request.
fn take_sign_out_flow_request(sign_out: &sign_out::SignOutState) -> bool {
    sign_out.flow_request.swap(false, Ordering::SeqCst)
}

/// Entry points 2 and 5: store the request, open Settings, and tell it (it also consumes on mount).
#[cfg(target_os = "macos")]
fn open_sign_out_flow(app: &tauri::AppHandle) {
    app.state::<AppState>().sign_out.flow_request.store(true, Ordering::SeqCst);
    show_macos_settings_window(app);
    let _ = app.emit_to("macos-settings", "menu:sign-out-flow", ());
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn request_sign_out_flow(app: tauri::AppHandle) -> Result<(), String> {
    open_sign_out_flow(&app);
    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn consume_sign_out_flow_request(state: State<'_, AppState>) -> bool {
    take_sign_out_flow_request(&state.sign_out)
}

/// W11: on macOS the switch step renders a snapshot, owned by the sign-in window, `onboarding`.
fn switch_snapshot(state: &AppState) -> Result<Option<sign_out::snapshot::AskCounts>, String> {
    #[cfg(target_os = "macos")]
    {
        let snapshot = count_ask("onboarding")?;
        // §6.4 (review I8): the switch's snapshot logs the same line as `sign_out_count`; it never sends (W11).
        tracing::info!(
            files = snapshot.counts.files,
            copied_files = snapshot.counts.copied_files,
            others = snapshot.counts.others,
            missing_files = snapshot.counts.missing_files,
            can_send = false,
            not_sending = "",
            earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
            "sign-out flow counted"
        );
        let counts = snapshot.counts.clone();
        state.sign_out.snapshots.record(snapshot);
        Ok(Some(counts))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Ok(None)
    }
}
```

Non-macOS twins of the two commands: `request_sign_out_flow` returns `Err("This works on a Mac only.".into())`, `consume_sign_out_flow_request` returns `false`. Register both.

The menu's `SignOut` arm calls `clear_session_impl(&state, false, None)` and gains, as the first arm of its `match &result`:

```rust
                    // Spec 2026-10-10 §4 row 5: a waiting change opens the flow in Settings; never the error dialog.
                    #[cfg(target_os = "macos")]
                    Err(error) if error.message.starts_with("unsent_changes: ") => open_sign_out_flow(&app),
```

`AccountMismatchDto` gains `#[serde(skip_serializing_if = "Option::is_none")] unsent: Option<sign_out::snapshot::AskCounts>`; `LoginOutcome::account_mismatch(pending_changes, unsent)` fills it; `SignInSettlement::AccountMismatch` gains `unsent: Option<sign_out::snapshot::AskCounts>`; `settle_sign_in`'s `DifferentAccount` arm becomes:

```rust
        reauth::SignInKind::DifferentAccount => {
            // A count that cannot be read settles nothing, as the switch's own count already does (triage 25).
            let unsent = switch_snapshot(state).map_err(ReauthError::before_store)?;
            Ok(SignInSettlement::AccountMismatch { pending_changes, unsent })
        }
```

The two callers pass `unsent` through (`LoginOutcome::account_mismatch(pending_changes, unsent)`). In `browser_login.rs`:

```rust
            Some(crate::SignInSettlement::AccountMismatch { pending_changes, unsent }) => {
                let _ = crate::revoke_desktop_session(&client, &base_url, &creds.session_token).await;
                let mut payload = serde_json::json!({ "pending_changes": pending_changes });
                if let Some(unsent) = unsent {
                    payload["unsent"] = serde_json::to_value(unsent).unwrap_or(serde_json::Value::Null);
                }
                emit(app, "account_mismatch", payload);
                return Err("account_mismatch".to_string());
            }
```

Every test call of `LoginOutcome::account_mismatch(n)` becomes `LoginOutcome::account_mismatch(n, None)` (`grep -n "account_mismatch(" src-tauri/src/lib.rs`); their expected JSON is unchanged.

- [ ] **Step 4: Run, expect PASS.** The three filters: `ok. 1` each. Then the existing login-outcome JSON test (filter `account_mismatch`): its baseline count.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Fill `unsent` from `pending_changes_count` (`unsent: None` and the frontend reads `pending_changes`): U19 red on `unsent`.
  2. Start a phase in `switch_snapshot`: U19 red on "no send is started".
  3. Handle `unsent_changes` as an error in the menu (delete the flow arm): U21 red.
  4. Store requests in a counter: U21 red on "not two".

- [ ] **Step 6: The task gate** with `<N>` = 13 and `<L>` = `L0 + 84`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs src-tauri/src/browser_login.rs
git commit -m "feat(macos): the menu, the compact page and the account switch reach the sign-out ask (1887)" -- src-tauri/src/sign_out/mod.rs src-tauri/src/lib.rs src-tauri/src/browser_login.rs
```

### Task 14: The R10 reset keeps before it purges

**Files:**
- Modify: `src-tauri/src/lib.rs`: `keep_for_sign_out` (Task 8) becomes `keep_in(db, acknowledge)` plus two callers; `StateDbLocalData` (`@184:lib.rs:5852-5857`) and its `reset` (`:5921-5934`); `bind_data_to_session` (`:5965-5985`); `spawn_bound_engine` (`:6229`); `SignOutState.reset_kept`
- Test: `src-tauri/src/lib.rs` `mod sign_out_keep_tests`

**Interfaces:**
- Consumes: `keep_unsent`, `Acknowledge::Everything` (Task 5); `purge_local_state_files_in(…, kept)` (Task 6); `sign_out_alert`, `KeptUnsent` (Tasks 8, 10).
- Produces: `fn keep_in(db: &state_db::StateDb, acknowledge: Acknowledge<'_>) -> Result<KeptSet, String>` (macOS); `StateDbLocalData.kept: std::sync::Mutex<Option<KeptSet>>`; `SignOutState.reset_kept: std::sync::Mutex<Option<(String, usize)>>` (the folder and its file count; `sign_out/mod.rs` cannot name `lib.rs`'s `KeptUnsent`); `fn surface_reset_folder(app: &tauri::AppHandle, folder: &str, files: usize)`.

- [ ] **Step 1: Write the failing test** in `sign_out_keep_tests`:

```rust
    /// U20.
    #[tokio::test]
    async fn the_r10_reset_keeps_before_it_purges() {
        with_the_shared_state_dir(async {
            let state_dir = state_paths::init_for_test();
            let sync_root = tempfile::tempdir().unwrap();
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"another account's save");
            queue_rename(&db, "r1");
            let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let logs = capture_logs_async(async {
                account_binding::LocalData::reset(&data).unwrap();
            })
            .await;
            let kept = data.kept.lock().unwrap().clone().expect("the reset kept the write");
            let folder = kept.folder.clone().unwrap();
            assert_eq!(std::fs::read(folder.join("f1.bin")).unwrap(), b"another account's save");
            assert_eq!(DesktopConfig::load().unwrap().kept_unsent_folder, Some(folder.to_string_lossy().into_owned()), "recorded");
            assert!(kept.others_acknowledged.contains("r1"));
            assert!(logs.contains("others_dropped=1"), "the dropped rename is counted in the reset's line: {logs}");
            assert!(db.waiting_rows().unwrap().is_empty(), "then the purge ran, with the guard");
            let production = super::finder_setup_command_tests::production_source();
            let reset = super::finder_setup_command_tests::body_between(&production, "    fn reset(&self) -> Result<(), String> {", "\n    }\n");
            assert!(reset.contains("purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref())"), "the guard is in the reset's path: {reset}");
            // A failing copy: the reset fails, nothing is deleted, no record.
            use std::os::unix::fs::PermissionsExt;
            DesktopConfig::update(|cfg| {
                cfg.kept_unsent_folder = None;
                cfg.kept_unsent_unfinished.clear();
                (true, ())
            })
            .unwrap();
            let staged = queue_write(&db, "w2", "f2", b"bytes");
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
            let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
            let failed = account_binding::LocalData::reset(&data);
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(failed.unwrap_err().starts_with("unsent_copy_failed: "));
            assert_eq!(db.waiting_rows().unwrap().len(), 1, "nothing deleted");
            assert_eq!(DesktopConfig::load().unwrap().kept_unsent_folder, None, "no record");
            reset_scratch(&db);
        })
        .await;
    }
```

(`capture_logs_async` is the async twin of Task 10's `capture_logs`; copy `engine_bridge.rs`'s `capture_logs_async`, `@184:engine_bridge.rs:16203`, into this module.)

- [ ] **Step 2: Run it, expect FAIL.** Filter `the_r10_reset_keeps`. Expected: compile error (`data.kept`), then the folder assertion.

- [ ] **Step 3: Implement.** Split Task 8's `keep_for_sign_out`:

```rust
/// The keep step on `db` (§5.2 step 7, W12): the kept root, the config, the clone and the durable save are the app's.
#[cfg(target_os = "macos")]
fn keep_in(db: &state_db::StateDb, acknowledge: sign_out::keep::Acknowledge<'_>) -> Result<sign_out::kept_set::KeptSet, String> {
    let failed = || sign_out::copy::coded(sign_out::copy::UNSENT_COPY_FAILED_CODE, sign_out::copy::E2_UNSENT_COPY_FAILED);
    let root = sign_out_kept_root().map_err(|_| failed())?;
    let config = DesktopConfig::path().map_err(|_| failed())?;
    sign_out::keep::keep_unsent(sign_out::keep::KeepInputs {
        db,
        acknowledge,
        root: &root,
        event_name: sign_out::kept_names::event_folder_name(chrono::Local::now()),
        config_path: &config,
        clone: &sign_out::kept_writer::SystemClone,
        config_fs: &config::RealConfigFs,
    })
    .map_err(|failure| failure.message())
}
```

`keep_for_sign_out` keeps its signature and calls `keep_in(&db, acknowledge)` after opening the database. `StateDbLocalData` gains `kept: std::sync::Mutex<Option<sign_out::kept_set::KeptSet>>` (initialised `Default::default()` in `for_engine_start`), and `reset` becomes:

```rust
    /// W12: keep (no send, no ask: every missing write and other change is acknowledged), then the purge with the
    /// guard, then the rows. A failed keep fails the reset, and nothing is deleted.
    fn reset(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let kept = Some(keep_in(&self.db, sign_out::keep::Acknowledge::Everything)?);
        #[cfg(not(target_os = "macos"))]
        let kept: Option<sign_out::kept_set::KeptSet> = None;
        let summary = purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref())?;
        tracing::info!(
            queued_ops_purged = summary.queued_ops_purged,
            files_removed = summary.files_removed,
            files_skipped = summary.files_skipped,
            others_dropped = kept.as_ref().map_or(0, |kept| kept.others_acknowledged.len()),
            "R10: local data reset before sync started"
        );
        #[cfg(target_os = "macos")]
        if let Err(error) = DesktopConfig::path().and_then(|path| sign_out::kept_record::clear_unfinished_at(&path)) {
            tracing::warn!(error = %error, "could not clear the unfinished sign-out marker");
        }
        *self.kept.lock().unwrap_or_else(|p| p.into_inner()) = kept;
        self.db
            .clear_account_data(cfg!(target_os = "macos"))
            .map_err(|e| format!("clear the local data: {e}"))
    }
```

In `bind_data_to_session`, in the `Bound::Reset` arm, hand the folder to the state for surfacing: `if let Some(kept) = data.kept.lock().unwrap_or_else(|p| p.into_inner()).take() { if let Some(folder) = kept.folder { *state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()) = Some((folder.to_string_lossy().into_owned(), kept.files)); } }`. In `spawn_bound_engine`, which holds the `app`, after `start_engine_bound` returns (either way):

```rust
    // W12: the R10 reset's kept folder is surfaced the way the app-start sweep surfaces macOS's: the row, then one alert.
    if let Some((folder, files)) = state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()).take() {
        surface_reset_folder(&app, &folder, files);
    }
```

with

```rust
fn surface_reset_folder(app: &tauri::AppHandle, folder: &str, files: usize) {
    emit_kept_folder_changed(app);
    if let Some((title, message)) = sign_out::copy::sign_out_alert(Some((folder, files)), None) {
        app.dialog().message(message).title(title).kind(tauri_plugin_dialog::MessageDialogKind::Info).show(|_| {});
    }
}
```

`app` is moved into the spawn closure in `spawn_bound_engine`; clone it first (`let surface_app = app.clone();`) and use the clone here.

- [ ] **Step 4: Run, expect PASS.** Filter `the_r10_reset_keeps`: `ok. 1 passed`. Then `account_binding` and `every_engine_start_is_bound_first`: their baseline counts.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Reset without the keep step (`let kept = None` on macOS): the guard is never asked, the write is purged unkept: U20 red on "the reset kept the write".
  2. Purge without passing the `KeptSet` (`purge_local_state_files_in(…, None)`): U20 red on "the guard is in the reset's path" (with a full keep the guard has nothing to refuse, so behaviour alone cannot show it).
  3. Ignore the copy failure (`unwrap_or_default()` on `keep_in`): U20 red on "nothing deleted".

- [ ] **Step 6: The task gate** with `<N>` = 14 and `<L>` = `L0 + 85`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src-tauri/src/lib.rs src-tauri/src/sign_out/mod.rs
git commit -m "feat(macos): resetting another account's local data keeps its waiting saves first (1887)" -- src-tauri/src/lib.rs src-tauri/src/sign_out/mod.rs
```

### Task 15: Design first: Dialog 3, dialog 4, spec A

Spec §11 "The design changes first": this commit lands before any frontend code, and the frontend pull request does not merge without it.

**Files:**
- Modify: `docs/specs/2026-10-02-macos-settings-dialogs.md` (Dialog 3, `:59-72`)
- Modify: `design/hifi/macos-settings-dialogs.html` (section 1, `:198-225`; section 4, `:310-321`; the copy table, `:351-360`)
- Modify: `docs/specs/2026-10-06-macos-finder-setup-reconciler.md` (the switch row, `:80`)

**Interfaces:** none in code. Produces the drawings Tasks 17 and 18 build to; every string is the spec's §11, character for character.

- [ ] **Step 1: Amend Dialog 3** in `docs/specs/2026-10-02-macos-settings-dialogs.md`, after its "Amendment — 10 Oct 2026 (task 1882)" bullet:

```markdown
- **Amendment — 11 Oct 2026 (task 1887, spec `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`):** "Sign out" in this sheet no longer signs out at once. It counts what waits (§5.1 there), then:
  - **nothing waits:** the sign-out runs, as before;
  - **changes wait and the app can send:** the sheet becomes the send phase: title “Sending changes first”, body “Sending {n} changes before signing out…” (“Sending 1 change before signing out…”) and the hint “This takes at most a minute.”, one button “Stop waiting”. Esc is “Stop waiting”. The count follows the events and never reads 0;
  - **changes still wait, or none could be sent:** the ask: title “Sign out with changes not sent?”, then A2, A3, A3e (with its folder paths in mono, one per line), A4 and at most one A5 line, each left out when its number is 0; buttons “Cancel” / “Sign out” (red, as this dialog's shipped treatment). Enter on open resolves to Cancel; Esc cancels;
  - **closing Settings** at the count, the send phase or the ask ends the flow with nothing changed; Settings opens next time on the Account tab with no sheet.
- After a sign-out that kept files, one alert, “Files kept on this Mac”, names our folder first (“Beebeeb copied {n} files with changes that hadn’t reached the server to this folder on this Mac:”), then macOS's folder with 1882's sentence, unchanged.
```

- [ ] **Step 2: Draw them** in `design/hifi/macos-settings-dialogs.html`. In section 1, after the two existing figures, add a `grid2` with four `figure.shot`s built from the classes the file already uses (`modal`, `modal-head`, `modal-title`, `modal-x`, `modal-body`, `sheet-copy`, `modal-foot`, `ms-btn`, `ms-btn--danger`):
  1. **Send phase:** title “Sending changes first”; body “Sending 3 changes before signing out…” and, as a second `p.sheet-copy`, “This takes at most a minute.”; footer one `ms-btn` “Stop waiting”. Caption: “Esc is Stop waiting. The count follows the app's events and never shows 0.”
  2. **Ask, files and other changes, offline:** title “Sign out with changes not sent?”; paragraphs: “2 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies them to a folder on this Mac and shows you where.” / “1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Signing out drops it, and that item stays as it is on the server.” / “This Mac can’t reach the server right now.”; footer Cancel / red Sign out. Caption: “Enter on open is Cancel; Esc cancels.”
  3. **Ask, a missing copy and an earlier folder:** “1 file you changed in Finder hasn’t reached the server.” / “Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.” / “A sign-out that didn’t finish copied changes to this folder, so a change listed as missing may be there:” then a `p.sheet-copy` in mono with `/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12`.
  4. **The alert, two folders:** a plain box titled “Files kept on this Mac” with “Beebeeb copied 2 files with changes that hadn’t reached the server to this folder on this Mac:”, the path, a blank line, “Files that hadn’t reached your vault yet were kept on this Mac, in this folder:”, macOS's path. Caption: “Drawn by macOS, not a Beebeeb window; ours first.”

  In section 4 (the switch step), replace the “changes waiting” card's copy in both variants (the onboarding Card at `:314` and the modal at `:319`): title “Switch accounts with changes not sent?”, then “This Mac is signed in to another Beebeeb account. Switching signs that account out of this Mac.”, then “1 file changed in Finder while that account was signed in hasn’t reached the server. Before switching, Beebeeb copies it to a folder on this Mac and shows you where.”, then “1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Switching drops it, and that item stays as it is on the server.” Update both captions so neither says “removes it” or “removes them”, and add “amended 11 Oct 2026 (task 1887)” to the section's comment line at `:147`. The “nothing waiting” variants stay as they are. In the copy table, add rows for the send phase, the ask and the alert with the strings above.

- [ ] **Step 3: Amend spec A** at `docs/specs/2026-10-06-macos-finder-setup-reconciler.md:80`: after “an account switch: the pending-edits warning first, then a full sign-out (**absent**, queue and cache purged), then a fresh sign-in (**present**)”, add “On macOS the warning is the ask of spec 2026-10-10 (A0s-A4s), and the sign-out keeps every waiting Finder save in a kept folder before the queue is purged; “queue and cache purged” stays true for what remains. — lead, 2026-10-11 (1887)”.

- [ ] **Step 4: Check the drawing.** Open `design/hifi/macos-settings-dialogs.html` in a local browser (a `file://` URL; no network), capture section 1 and section 4 in light and dark into `$EVID/t15-section1-light.png`, `t15-section1-dark.png`, `t15-section4-light.png`, `t15-section4-dark.png`. Then:

```bash
cd $WT && grep -c "removes them\|removes it" design/hifi/macos-settings-dialogs.html
grep -c "Sending changes first\|Sign out with changes not sent?\|Switch accounts with changes not sent?" design/hifi/macos-settings-dialogs.html
```

Expected: the first count is the number of occurrences left only in the "nothing waiting" history or in struck notes (write it into the Notes, with each line's context); the second is at least 4.

- [ ] **Step 5: Commit** (docs and design only; the lead reviews the drawings before Task 16 starts).

```bash
cd $WT && git add -- docs/specs/2026-10-02-macos-settings-dialogs.md design/hifi/macos-settings-dialogs.html docs/specs/2026-10-06-macos-finder-setup-reconciler.md
git commit -m "docs(design): draw the sign-out send phase, the ask and the switch step before the code (1887)" -- docs/specs/2026-10-02-macos-settings-dialogs.md design/hifi/macos-settings-dialogs.html docs/specs/2026-10-06-macos-finder-setup-reconciler.md
```

### Task 16: Frontend copy and its mirror

**Files:**
- Create: `src/signOutCopy.ts`
- Test: `tests/signOutCopy.test.ts`

**Interfaces:**
- Consumes: `src-tauri/src/sign_out/copy.rs` (Task 10) through `tests/fixtures/rustConstants.ts` (`rustStr`).
- Produces (`src/signOutCopy.ts`): every constant Task 10 lists, same names and same text; the types `NotSending`, `EndReason`, `AskCounts`, `SignOutCount`, `SendEnded`; the builders `progressLine(n)`, `filesLine(n, c, isSwitch)`, `missingLine(n, k)`, `earlierFoldersLine(count)`, `othersLine(m, either, isSwitch)`, `reasonLine(reason)`, `keptAlertLine(n)`, and `askLines(ask: AskCounts, reason: EndReason | NotSending | null, isSwitch: boolean): { paragraphs: string[]; foldersAfter: number; folders: string[]; foldersLine: string | null }` (A3e and its paths render after the first `foldersAfter` paragraphs, so under A3, spec §11); `stripCode(message: string): string` (removes a leading `<code>: `).

- [ ] **Step 1: Write the failing tests** in `tests/signOutCopy.test.ts`:

```ts
import { describe, expect, test } from 'bun:test'
import * as copy from '../src/signOutCopy'
import { rustStr } from './fixtures/rustConstants'

const ask = (over: Partial<copy.AskCounts> = {}): copy.AskCounts => ({
  askId: 'a', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], ...over,
})

describe('the sign-out copy (spec 2026-10-10 §11)', () => {
  /** F6: every string equals Rust's. */
  test('the TypeScript constants equal the Rust ones', () => {
    const names = [
      'P0_TITLE', 'P1_ONE', 'P1_MANY', 'P2_HINT', 'P3_STOP', 'A0_TITLE', 'A0S_TITLE', 'A1S_LINE', 'A2_ONE', 'A2_MANY',
      'A2_COPY_IT', 'A2_COPY_THEM', 'A2_COPY_SOME', 'A2S_ONE', 'A2S_MANY', 'A2S_COPY_IT', 'A2S_COPY_THEM', 'A2S_COPY_SOME',
      'A3_ONE_OF_ONE', 'A3_ONE_OF_MANY', 'A3_MANY', 'A3E_ONE', 'A3E_MANY', 'A4_ONE', 'A4_MANY', 'A4S_ONE', 'A4S_MANY',
      'A5_OFFLINE', 'A5_SESSION', 'A5_PAUSED', 'A5_HELD', 'A5_ENGINE_STOPPED', 'A5_TIME_LIMIT', 'A5_ACCOUNT',
      'ASK_CANCEL', 'ASK_SIGN_OUT', 'ASK_SWITCH', 'K0_TITLE', 'K1_ONE', 'K1_MANY', 'K2_ROW', 'K2_SHOW', 'K2_DISMISS',
      'K3_TOAST', 'E1_UNSENT_CHANGES', 'E2_UNSENT_COPY_FAILED', 'E3_UNSENT_COUNT_FAILED',
    ] as const
    for (const name of names) {
      expect((copy as Record<string, unknown>)[name], name).toBe(rustStr('sign_out/copy.rs', name))
    }
  })

  /** F2. */
  test('the ask reads exactly, for every case', () => {
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1 }), null, false).paragraphs).toEqual([
      '1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.',
    ])
    expect(copy.askLines(ask({ others: 2 }), 'offline', false).paragraphs).toEqual([
      '2 other changes (renames, moves, deletions, new folders or restores) haven’t reached the server. Signing out drops them, and those items stay as they are on the server.',
      'This Mac can’t reach the server right now.',
    ])
    expect(copy.askLines(ask({ files: 3, copiedFiles: 2, others: 1 }), 'time_limit', false).paragraphs).toEqual([
      '3 files you changed in Finder haven’t reached the server. Before signing out, Beebeeb copies 2 of them to a folder on this Mac and shows you where.',
      '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Signing out drops it, and that item stays as it is on the server.',
      'Sending took longer than a minute. Choose Cancel to let it finish, then sign out again.',
    ])
    // n = k = c = 1: an earlier-build write is missing, a newer one is copied: A2 keeps its second sentence, then A3.
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1, missingFiles: 1 }), null, false).paragraphs).toEqual([
      '1 file you changed in Finder hasn’t reached the server. Before signing out, Beebeeb copies it to a folder on this Mac and shows you where.',
      'Beebeeb’s copy of a change to that file is missing, so that change can’t be kept.',
    ])
    expect(copy.askLines(ask({ files: 2, missingFiles: 2 }), null, false).paragraphs[0]).toBe('2 files you changed in Finder haven’t reached the server.')
    // A3e: none, one path, two paths (oldest first, one per line).
    expect(copy.askLines(ask({ files: 1, missingFiles: 1 }), null, false).foldersLine).toBeNull()
    const one = copy.askLines(ask({ files: 1, missingFiles: 1, earlierFolders: ['/k/1'] }), null, false)
    expect([one.foldersLine, one.folders]).toEqual([copy.A3E_ONE, ['/k/1']])
    expect(one.foldersAfter).toBe(2) // after A2 and A3, before A4 and A5
    const two = copy.askLines(ask({ files: 1, missingFiles: 1, earlierFolders: ['/k/1', '/k/2'] }), null, false)
    expect([two.foldersLine, two.folders]).toEqual([copy.A3E_MANY, ['/k/1', '/k/2']])
    // Each reason, and none for `stopped`.
    for (const [reason, line] of [['offline', copy.A5_OFFLINE], ['session', copy.A5_SESSION], ['paused', copy.A5_PAUSED], ['held', copy.A5_HELD], ['engine_stopped', copy.A5_ENGINE_STOPPED], ['time_limit', copy.A5_TIME_LIMIT], ['account', copy.A5_ACCOUNT]] as const) {
      expect(copy.reasonLine(reason)).toBe(line)
    }
    expect(copy.reasonLine('stopped')).toBeNull()
    // The switch: its own sentences, and no reason line.
    expect(copy.askLines(ask({ files: 1, copiedFiles: 1, others: 1 }), 'offline', true).paragraphs).toEqual([
      '1 file changed in Finder while that account was signed in hasn’t reached the server. Before switching, Beebeeb copies it to a folder on this Mac and shows you where.',
      '1 other change (a rename, move, deletion, new folder or restore) hasn’t reached the server either. Switching drops it, and that item stays as it is on the server.',
    ])
  })

  /** F7. */
  test('a progress line never reads 0', () => {
    expect(copy.progressLine(0)).toBeNull()
    expect(copy.progressLine(1)).toBe('Sending 1 change before signing out…')
    expect(copy.progressLine(4)).toBe('Sending 4 changes before signing out…')
  })

  test('a code prefix is never shown to a person', () => {
    expect(copy.stripCode(`unsent_copy_failed: ${copy.E2_UNSENT_COPY_FAILED}`)).toBe(copy.E2_UNSENT_COPY_FAILED)
    expect(copy.stripCode('Sign-out paused: x')).toBe('Sign-out paused: x')
  })
})
```

- [ ] **Step 2: Run it, expect FAIL.** `cd $WT && bun test tests/signOutCopy.test.ts > $EVID/t16-red.log 2>&1; grep -E "pass|fail" $EVID/t16-red.log | tail -3`. Expected: a module-not-found failure.

- [ ] **Step 3: Implement** `src/signOutCopy.ts`. Each constant is the Rust string of the same name, copied character for character (the test checks it). Then:

```ts
export type NotSending = 'engine_stopped' | 'session' | 'offline' | 'paused' | 'held'
export type EndReason = 'all_sent' | 'held' | 'time_limit' | 'stopped' | 'closed' | 'offline' | 'session' | 'engine_stopped' | 'account'
export interface AskCounts { askId: string; files: number; copiedFiles: number; others: number; missingFiles: number; earlierFolders: string[] }
export interface SignOutCount extends AskCounts { canSend: boolean; notSending: NotSending | null }
export interface SendEnded { reason: EndReason; ask: AskCounts | null }

const fill = (template: string, key: string, value: number) => template.replace(key, String(value))

/** P1, never for 0 (W2). */
export function progressLine(n: number): string | null {
  if (n <= 0) return null
  return n === 1 ? P1_ONE : fill(P1_MANY, '{n}', n)
}

/** A2 or A2s: the count, then the copy sentence by c; left out for c = 0 (re-review N1). */
export function filesLine(n: number, c: number, isSwitch: boolean): string | null {
  if (n <= 0) return null
  const [one, many, it, them, some] = isSwitch
    ? [A2S_ONE, A2S_MANY, A2S_COPY_IT, A2S_COPY_THEM, A2S_COPY_SOME]
    : [A2_ONE, A2_MANY, A2_COPY_IT, A2_COPY_THEM, A2_COPY_SOME]
  const first = n === 1 ? one : fill(many, '{n}', n)
  if (c <= 0) return first
  const second = c === n ? (n === 1 ? it : them) : fill(some, '{c}', c)
  return `${first} ${second}`
}

export function missingLine(n: number, k: number): string | null {
  if (k <= 0) return null
  if (k === 1) return n === 1 ? A3_ONE_OF_ONE : A3_ONE_OF_MANY
  return fill(A3_MANY, '{k}', k)
}

export function earlierFoldersLine(count: number): string | null {
  if (count <= 0) return null
  return count === 1 ? A3E_ONE : A3E_MANY
}

export function othersLine(m: number, either: boolean, isSwitch: boolean): string | null {
  if (m <= 0) return null
  const [one, many] = isSwitch ? [A4S_ONE, A4S_MANY] : [A4_ONE, A4_MANY]
  return (m === 1 ? one : fill(many, '{m}', m)).replace('{either}', either ? ' either' : '')
}

export function reasonLine(reason: EndReason | NotSending | null): string | null {
  switch (reason) {
    case 'offline': return A5_OFFLINE
    case 'session': return A5_SESSION
    case 'paused': return A5_PAUSED
    case 'held': return A5_HELD
    case 'engine_stopped': return A5_ENGINE_STOPPED
    case 'time_limit': return A5_TIME_LIMIT
    case 'account': return A5_ACCOUNT
    default: return null
  }
}

export function keptAlertLine(n: number): string | null {
  if (n <= 0) return null
  return n === 1 ? K1_ONE : fill(K1_MANY, '{n}', n)
}

/** The ask's body, in order: A2, A3, A4, A5 (the switch shows no reason line). A3e's sentence and paths go under A3:
 *  after the first `foldersAfter` paragraphs. */
export function askLines(ask: AskCounts, reason: EndReason | NotSending | null, isSwitch: boolean) {
  const files = filesLine(ask.files, ask.copiedFiles, isSwitch)
  const head = [files, missingLine(ask.files, ask.missingFiles)].filter((line): line is string => line !== null)
  const tail = [othersLine(ask.others, files !== null, isSwitch), isSwitch ? null : reasonLine(reason)].filter(
    (line): line is string => line !== null,
  )
  const folders = ask.missingFiles > 0 ? ask.earlierFolders : []
  return { paragraphs: [...head, ...tail], foldersAfter: head.length, folders, foldersLine: earlierFoldersLine(folders.length) }
}

/** §11: a caller matches the code; a person reads the sentence. */
export function stripCode(message: string): string {
  return message.replace(/^(unsent_changes|unsent_copy_failed|unsent_count_failed): /, '')
}
```

- [ ] **Step 4: Run, expect PASS.** `bun test tests/signOutCopy.test.ts`: `4 pass`, `0 fail`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Change one character of `A2_MANY`: the mirror test red.
  2. Drop `' either'` (always `''`): F2 red on the time-limit case.
  3. Leave A2's second sentence out when `k === n` (`if (c <= 0 || ask.missingFiles === n)`): F2 red on the n = k = c = 1 case.
  4. Show A3e when `earlierFolders` is empty: F2 red on `foldersLine` null.
  5. Use the singular sentence for two paths: F2 red.
  6. `progressLine(0)` returns `'Sending 0 changes…'`: F7 red.

- [ ] **Step 6: The frontend gate** with `<N>` = 16; expected `B0 + 4 pass`, `0 fail`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src/signOutCopy.ts tests/signOutCopy.test.ts
git commit -m "feat(macos): the sign-out ask's copy, mirrored from Rust and pinned equal (1887)" -- src/signOutCopy.ts tests/signOutCopy.test.ts
```

### Task 17: The Settings flow

**Files:**
- Create: `src/signOutFlow.ts` (the flow, its API and its two event connectors)
- Modify: `src/desktopApi.ts:1132-1134` (`clearSession` takes `keepUnsent`)
- Modify: `src/MacSettings.tsx`: `AccountTab` (`@184:src/MacSettings.tsx:250-490`) runs the flow; two new sheets, `SendingSheet` and `UnsentAskSheet`, beside `ConfirmSheet`; the root (`:924-952`) consumes flow requests
- Test: `tests/signOutFlow.test.ts` (create); `tests/macSettingsTabs.test.tsx` (its `backend()` and `baseBindings` learn the new commands and names)

**Interfaces:**
- Consumes: `signOutCopy` (Task 16); commands `sign_out_count`, `sign_out_send_start`, `sign_out_send_stop`, `clear_session` with `keepUnsent`, `sign_out_result_ack`, `consume_sign_out_flow_request`; events `sign-out-send-progress`, `sign-out-send-ended`, `sign-out-flow-closed`, `menu:sign-out-flow` (Tasks 11-13).
- Produces (`src/signOutFlow.ts`): `FlowView`, `FlowOutcome`, `SignOutFlowApi`, `signOutFlowApi`, `class SignOutFlow` with `open()`, `confirm()`, `request()`, `cancel()`, `stopWaiting()`, `progress(n)`, `ended(event)`, `signOutAtAsk()`, `reset()`, `current()`; `connectSignOutFlowEvents(listen, flow)`, `connectSignOutFlowMenu(listen, consume, onRequest)`. `clearSession(options: { forgetEmail?: boolean; keepUnsent?: { askId: string } })`.

- [ ] **Step 1: Write the failing tests** in `tests/signOutFlow.test.ts`:

```ts
import { describe, expect, test } from 'bun:test'
import type { CommandResult, SessionActionOutcome } from '../src/desktopApi'
import type { AskCounts, SignOutCount } from '../src/signOutCopy'
import { SignOutFlow, connectSignOutFlowMenu, type FlowOutcome, type FlowView, type SignOutFlowApi } from '../src/signOutFlow'

const ok = <T,>(value: T): CommandResult<T> => ({ ok: true, value })
const err = (reason: string): CommandResult<never> => ({ ok: false, reason, unsupported: false })
const asked = (askId: string, over: Partial<SignOutCount> = {}): SignOutCount => ({
  askId, files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [], canSend: true, notSending: null, ...over,
})
const nothing = (askId: string): SignOutCount => asked(askId, { files: 0, copiedFiles: 0, canSend: false })

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}

/** A scripted backend: answers are queued per command; a `deferred` answer waits for the test. */
function harness(script: { count?: Array<CommandResult<SignOutCount> | Promise<CommandResult<SignOutCount>>>; clear?: Array<CommandResult<SessionActionOutcome> | Promise<CommandResult<SessionActionOutcome>>> }) {
  const calls: Array<{ name: string; args?: unknown }> = []
  const views: FlowView[] = []
  const outcomes: FlowOutcome[] = []
  const order: string[] = []
  const counts = [...(script.count ?? [])]
  const clears = [...(script.clear ?? [])]
  const api: SignOutFlowApi = {
    count: async () => { calls.push({ name: 'count' }); return counts.shift()! },
    sendStart: async (askId) => { calls.push({ name: 'sendStart', args: askId }); return ok(undefined) },
    sendStop: async () => { calls.push({ name: 'sendStop' }); return ok(undefined) },
    clearSession: async (keepUnsent) => { calls.push({ name: 'clearSession', args: keepUnsent }); return clears.shift()! },
    acknowledge: async () => { calls.push({ name: 'acknowledge' }); order.push('ack'); return ok(undefined) },
  }
  const flow = new SignOutFlow(api, (view) => views.push(view), (outcome) => { outcomes.push(outcome); order.push('shown') })
  return { flow, calls, views, outcomes, order, names: () => calls.map((c) => c.name) }
}
const tick = () => new Promise((r) => setTimeout(r, 0))

describe('the Settings sign-out flow (spec 2026-10-10 §5.1)', () => {
  /** F1. */
  test('count, send, ask from the phase\'s own snapshot, then sign out with that askId', async () => {
    const h = harness({ count: [ok(asked('first'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names()).toEqual(['count', 'sendStart'])
    expect(h.calls[1].args).toBe('first')
    h.flow.progress(1)
    expect(h.flow.current()).toEqual({ kind: 'sending', changesLeft: 1 })
    const second: AskCounts = { askId: 'second', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }
    await h.flow.ended({ reason: 'offline', ask: second })
    expect(h.flow.current()).toEqual({ kind: 'ask', ask: second, reason: 'offline' })
    await h.flow.signOutAtAsk()
    expect(h.calls.at(-1)).toEqual({ name: 'clearSession', args: { askId: 'second' } })
  })

  test('Cancel at the ask calls nothing', async () => {
    const h = harness({ count: [ok(asked('a', { canSend: false, notSending: 'offline' }))] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.cancel()
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    expect(h.names()).toEqual(['count'])
  })

  /** F3, the flow's half: Esc during the phase is "Stop waiting", never Cancel. */
  test('Esc during the send phase stops waiting', async () => {
    const h = harness({ count: [ok(asked('a'))] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.cancel()
    expect(h.flow.current().kind).toBe('sending')
    await h.flow.stopWaiting()
    expect(h.names()).toEqual(['count', 'sendStart', 'sendStop'])
  })

  /** F10. */
  test('a refusal after the phase goes straight to the ask, and the phase starts once', async () => {
    const h = harness({ count: [ok(asked('a')), ok(asked('b', { canSend: true }))], clear: [err('unsent_changes: Some changes…')] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.ended({ reason: 'all_sent', ask: null })
    expect(h.calls.find((c) => c.name === 'clearSession')?.args).toBeUndefined()
    expect(h.flow.current()).toMatchObject({ kind: 'ask', ask: { askId: 'b' } })
    expect(h.names().filter((name) => name === 'sendStart')).toHaveLength(1)
    expect(h.outcomes, 'no toast for the refusal').toEqual([])
  })

  /** F11. */
  test('a close ends the flow at every step, and late answers do nothing', async () => {
    // The count in flight.
    const late = deferred<CommandResult<SignOutCount>>()
    let h = harness({ count: [late.promise] })
    h.flow.open()
    const counting = h.flow.confirm()
    h.flow.reset()
    late.resolve(ok(asked('a')))
    await counting
    expect(h.names()).toEqual(['count'])
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    // The send phase ends `closed`: no ask, no clear_session.
    h = harness({ count: [ok(asked('a'))] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.ended({ reason: 'closed', ask: null })
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    expect(h.names()).not.toContain('clearSession')
    // At the ask; then "Sign out…" starts a new flow at the count.
    h = harness({ count: [ok(asked('a', { canSend: false })), ok(nothing('b'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    h.flow.reset()
    expect(h.flow.current()).toEqual({ kind: 'idle' })
    h.flow.open()
    await h.flow.confirm()
    expect(h.names()).toEqual(['count', 'count', 'clearSession'])
    // A clear_session answer after the close: no toast, no acknowledgement.
    const answer = deferred<CommandResult<SessionActionOutcome>>()
    h = harness({ count: [ok(asked('a', { canSend: false }))], clear: [answer.promise] })
    h.flow.open()
    await h.flow.confirm()
    const signing = h.flow.signOutAtAsk()
    h.flow.reset()
    answer.resolve(err('unsent_copy_failed: Beebeeb couldn’t copy…'))
    await signing
    await tick()
    expect(h.outcomes).toEqual([])
    expect(h.names()).not.toContain('acknowledge')
  })

  /** F12, the Settings caller. */
  test('the result is acknowledged once, after it was shown, and only when it needed showing', async () => {
    let h = harness({ count: [ok(asked('a', { canSend: false }))], clear: [err('Sign-out paused: x')] })
    h.flow.open()
    await h.flow.confirm()
    await h.flow.signOutAtAsk()
    await tick()
    expect(h.order).toEqual(['shown', 'ack'])
    h = harness({ count: [ok(nothing('a'))], clear: [ok({ warning: { code: 'finder_removal_unconfirmed', sentence: 'w' } as any })] })
    h.flow.open()
    await h.flow.confirm()
    await tick()
    expect(h.order).toEqual(['shown', 'ack'])
    h = harness({ count: [ok(nothing('a'))], clear: [ok({ warning: null })] })
    h.flow.open()
    await h.flow.confirm()
    await tick()
    expect(h.names()).not.toContain('acknowledge')
  })

  /** F9. */
  test('a flow request is consumed on mount and on the event', async () => {
    const handlers: Array<() => void> = []
    const listen = async (_event: string, handler: () => void) => { handlers.push(handler); return () => {} }
    const answers = [ok(false), ok(true)]
    let requests = 0
    connectSignOutFlowMenu(listen as any, async () => answers.shift()!, () => { requests += 1 })
    await tick()
    expect(requests, 'false changes nothing').toBe(0)
    handlers[0]()
    await tick()
    expect(requests, 'true starts the flow').toBe(1)
  })
})
```

In `tests/macSettingsTabs.test.tsx`: `backend()` gains `sign_out_count: () => ({ askId: 'a0', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null })` and `sign_out_result_ack: () => null`, and `baseBindings` gains every new name `AccountTab` and `MacSettings` reference (the compiler and the harness name them: `SignOutFlow`, `signOutFlowApi`, `connectSignOutFlowEvents`, `connectSignOutFlowMenu`, `SendingSheet`, `UnsentAskSheet`, `askLines`, `progressLine`, `stripCode`, and the copy constants), loaded the way `ConfirmSheet` is (`@184:tests/macSettingsTabs.test.tsx:33`). Add two tests there:

```ts
  /** F3, the sheets: Esc is Stop waiting in the phase and Cancel at the ask; Enter on open is Cancel. */
  test('the send-phase and ask sheets map Esc and Enter as Dialog 3 does', () => {
    const stops: string[] = []
    const sending = SendingSheet({ changesLeft: 2, onStop: () => stops.push('stop') })
    sending.props.onClose()
    expect(stops).toEqual(['stop'])
    const calls: string[] = []
    const sheet = UnsentAskSheet({ ask: { askId: 'a', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [] }, reason: null, onCancel: () => calls.push('cancel'), onSignOut: () => calls.push('sign out') })
    sheet.props.onClose()
    expect(calls, 'Esc cancels').toEqual(['cancel'])
    expect(sheet.props.initialFocusRef).toBeUndefined() // focus starts on the close button, so Enter on open is Cancel
  })

  test('nothing waits: "Sign out" counts, then signs out with no keepUnsent', async () => {
    const m = open('AccountTab', backend({ clear_session: (args: any) => ({ warning: null, args }) }))
    await settle(m)
    await press(m, 'Sign out…')
    await press(m, 'Sign out')
    await settle(m)
    const names = m.calls.map((c) => c.name)
    expect(names.indexOf('sign_out_count')).toBeLessThan(names.indexOf('clear_session'))
    expect(m.calls.find((c) => c.name === 'clear_session')?.args?.keepUnsent).toBeUndefined()
  })
```

- [ ] **Step 2: Run them, expect FAIL.** `bun test tests/signOutFlow.test.ts tests/macSettingsTabs.test.tsx > $EVID/t17-red.log 2>&1`. Expected: module not found, then the new Settings tests red.

- [ ] **Step 3: Implement.** `src/desktopApi.ts`:

```ts
export function clearSession(options: { forgetEmail?: boolean; keepUnsent?: { askId: string } } = {}): Promise<CommandResult<SessionActionOutcome>> {
  const args: Record<string, unknown> = {}
  if (options.forgetEmail) args.forgetEmail = true
  if (options.keepUnsent) args.keepUnsent = options.keepUnsent
  return sessionAction('clear_session', Object.keys(args).length ? args : undefined)
}
```

`src/signOutFlow.ts`:

```ts
/**
 * Spec 2026-10-10 §5.1 (task 1887): the sign-out flow in Settings › Account. Rust owns the snapshot, the phase and
 * its deadline; this owns only what the sheet shows. A close or reload (`sign-out-flow-closed`) resets it, and any
 * answer that arrives after a reset is dropped: no toast, no acknowledgement, because Rust shows what needs showing.
 */
import { clearSession, command, type CommandResult, type SessionActionOutcome } from './desktopApi'
import type { AskCounts, EndReason, NotSending, SendEnded, SignOutCount } from './signOutCopy'

export type FlowView =
  | { kind: 'idle' }
  | { kind: 'confirm' }
  | { kind: 'counting' }
  | { kind: 'sending'; changesLeft: number }
  | { kind: 'ask'; ask: AskCounts; reason: EndReason | NotSending | null }
  | { kind: 'signingOut' }

export type FlowOutcome = { kind: 'signedOut'; warning: string | null } | { kind: 'failed'; message: string }

export interface SignOutFlowApi {
  count(): Promise<CommandResult<SignOutCount>>
  sendStart(askId: string): Promise<CommandResult<void>>
  sendStop(): Promise<CommandResult<void>>
  clearSession(keepUnsent?: { askId: string }): Promise<CommandResult<SessionActionOutcome>>
  acknowledge(): Promise<CommandResult<void>>
}

export const signOutFlowApi: SignOutFlowApi = {
  count: () => command<SignOutCount>('sign_out_count'),
  sendStart: (askId) => command<void>('sign_out_send_start', { askId }),
  sendStop: () => command<void>('sign_out_send_stop'),
  clearSession: (keepUnsent) => clearSession({ keepUnsent }),
  acknowledge: () => command<void>('sign_out_result_ack'),
}

const REFUSED = 'unsent_changes: '

export class SignOutFlow {
  private view: FlowView = { kind: 'idle' }
  private generation = 0
  private sent = false

  constructor(
    private readonly api: SignOutFlowApi,
    private readonly show: (view: FlowView) => void,
    private readonly report: (outcome: FlowOutcome) => void,
  ) {}

  current(): FlowView {
    return this.view
  }

  private set(view: FlowView) {
    this.view = view
    this.show(view)
  }

  /** "Sign out…". */
  open(): void {
    if (this.view.kind === 'idle') this.set({ kind: 'confirm' })
  }

  /** The confirm sheet's "Sign out". */
  async confirm(): Promise<void> {
    if (this.view.kind === 'confirm') await this.count()
  }

  /** A flow request from the menu or the compact page: straight to the count. */
  async request(): Promise<void> {
    if (this.view.kind === 'idle' || this.view.kind === 'confirm') await this.count()
  }

  /** Cancel, or Esc at the confirm and at the ask. Esc during the phase is `stopWaiting`, never this. */
  cancel(): void {
    if (this.view.kind === 'confirm' || this.view.kind === 'ask') this.set({ kind: 'idle' })
  }

  async stopWaiting(): Promise<void> {
    if (this.view.kind === 'sending') await this.api.sendStop()
  }

  progress(changesLeft: number): void {
    if (this.view.kind === 'sending' && changesLeft > 0) this.set({ kind: 'sending', changesLeft })
  }

  async ended(event: SendEnded): Promise<void> {
    if (this.view.kind !== 'sending') return
    if (event.reason === 'closed') return this.reset()
    if (event.reason === 'all_sent') return this.signOut(undefined)
    if (event.ask) return this.set({ kind: 'ask', ask: event.ask, reason: event.reason === 'stopped' ? null : event.reason })
    return this.count()
  }

  /** The ask's "Sign out". */
  async signOutAtAsk(): Promise<void> {
    if (this.view.kind === 'ask') await this.signOut({ askId: this.view.ask.askId })
  }

  /** `sign-out-flow-closed`, or a phase that ended `closed`. */
  reset(): void {
    this.generation += 1
    this.sent = false
    this.set({ kind: 'idle' })
  }

  private async count(): Promise<void> {
    const generation = this.generation
    this.set({ kind: 'counting' })
    const counted = await this.api.count()
    if (generation !== this.generation) return
    if (!counted.ok) {
      this.set({ kind: 'idle' })
      this.report({ kind: 'failed', message: counted.reason })
      return
    }
    const ask = counted.value
    if (ask.files === 0 && ask.others === 0) return this.signOut(undefined)
    // §5.1 step 4: a phase starts at most once per flow, so an autosaving editor cannot loop the person.
    if (ask.canSend && !this.sent) {
      this.sent = true
      this.set({ kind: 'sending', changesLeft: ask.files + ask.others })
      const started = await this.api.sendStart(ask.askId)
      if (generation !== this.generation) return
      if (!started.ok) return this.count()
      return
    }
    this.set({ kind: 'ask', ask, reason: ask.notSending })
  }

  private async signOut(keepUnsent: { askId: string } | undefined): Promise<void> {
    const generation = this.generation
    this.set({ kind: 'signingOut' })
    const result = await this.api.clearSession(keepUnsent)
    if (generation !== this.generation) return
    if (!result.ok && result.reason.startsWith(REFUSED)) return this.count()
    this.sent = false
    this.set({ kind: 'idle' })
    const outcome: FlowOutcome = result.ok
      ? { kind: 'signedOut', warning: result.value.warning?.sentence ?? null }
      : { kind: 'failed', message: result.reason }
    this.report(outcome)
    // F12: after it was shown, and only for what Rust holds pending.
    if (outcome.kind === 'failed' || outcome.warning !== null) void this.api.acknowledge()
  }
}

type Listen = <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>

/** The phase's events and the close, for one mounted Account tab. Returns the cleanup. */
export function connectSignOutFlowEvents(listen: Listen, flow: SignOutFlow): () => void {
  const unlistens = [
    listen<{ changesLeft: number }>('sign-out-send-progress', (event) => flow.progress(event.payload.changesLeft)),
    listen<SendEnded>('sign-out-send-ended', (event) => void flow.ended(event.payload)),
    listen<null>('sign-out-flow-closed', () => flow.reset()),
  ]
  return () => {
    for (const unlisten of unlistens) void unlisten.then((stop) => stop())
  }
}

/** F9: consume a pending request on mount and on every `menu:sign-out-flow`. */
export function connectSignOutFlowMenu(listen: Listen, consume: () => Promise<CommandResult<boolean>>, onRequest: () => void): () => void {
  const take = async () => {
    const result = await consume()
    if (result.ok && result.value) onRequest()
  }
  void take()
  const unlisten = listen<null>('menu:sign-out-flow', () => void take())
  return () => void unlisten.then((stop) => stop())
}
```

In `MacSettings.tsx`, beside `ConfirmSheet`:

```tsx
/** Spec 2026-10-10 §11, P0-P3: the send phase. Esc is "Stop waiting". The count never reads 0. */
function SendingSheet({ changesLeft, onStop }: { changesLeft: number; onStop: () => void }) {
  return (
    <Modal open onClose={onStop} title={P0_TITLE} maxWidth={400} footer={<div className="ms-sheet-footer"><Btn onClick={onStop}>{P3_STOP}</Btn></div>}>
      <p className="ms-sheet-copy" aria-live="polite">{progressLine(changesLeft)}</p>
      <p className="ms-sheet-copy">{P2_HINT}</p>
    </Modal>
  )
}

/** Spec 2026-10-10 §11, A0-A5: the ask. Dialog 3's keyboard rule: Enter on open is Cancel, Esc cancels. */
function UnsentAskSheet({ ask, reason, onCancel, onSignOut }: { ask: AskCounts; reason: EndReason | NotSending | null; onCancel: () => void; onSignOut: () => void }) {
  const lines = askLines(ask, reason, false)
  const onConfirmKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    onSignOut()
  }
  return (
    <Modal
      open
      onClose={onCancel}
      title={A0_TITLE}
      maxWidth={440}
      footer={
        <div className="ms-sheet-footer">
          <Btn onClick={onCancel}>{ASK_CANCEL}</Btn>
          <Btn danger onKeyDown={onConfirmKeyDown} onClick={onSignOut}>{ASK_SIGN_OUT}</Btn>
        </div>
      }
    >
      {lines.paragraphs.slice(0, lines.foldersAfter).map((line) => <p key={line} className="ms-sheet-copy">{line}</p>)}
      {lines.foldersLine ? <p className="ms-sheet-copy">{lines.foldersLine}</p> : null}
      {lines.folders.map((folder) => <div key={folder} className="ms-mono ms-mono--wrap">{folder}</div>)}
      {lines.paragraphs.slice(lines.foldersAfter).map((line) => <p key={line} className="ms-sheet-copy">{line}</p>)}
    </Modal>
  )
}
```

In `AccountTab`: remove `confirmSignOut` and `signOut`; add

```tsx
  const [flowView, setFlowView] = useState<FlowView>({ kind: 'idle' })
  const flowRef = useRef<SignOutFlow | null>(null)
  if (flowRef.current === null) {
    flowRef.current = new SignOutFlow(signOutFlowApi, setFlowView, (outcome) => {
      if (outcome.kind === 'failed') {
        showToast({ variant: 'error', title: 'Couldn’t sign out', message: stripCode(outcome.message) })
      } else {
        holdSignOutWarning(outcome.warning)
      }
      void refresh()
    })
  }
  const flow = flowRef.current
  useEffect(() => connectSignOutFlowEvents(listen, flow), [flow])
  useEffect(() => {
    if (flowRequest > 0) void flow.request()
  }, [flowRequest, flow])
```

`AccountTab` takes `{ flowRequest }: { flowRequest: number }`; the "Sign out…" button calls `flow.open()` and is disabled while `flowView.kind !== 'idle'`; the confirm sheet is `open={flowView.kind === 'confirm'}`, `onCancel={() => flow.cancel()}`, `onConfirm={() => void flow.confirm()}`; then:

```tsx
      {flowView.kind === 'sending' ? <SendingSheet changesLeft={flowView.changesLeft} onStop={() => void flow.stopWaiting()} /> : null}
      {flowView.kind === 'ask' ? (
        <UnsentAskSheet ask={flowView.ask} reason={flowView.reason} onCancel={() => flow.cancel()} onSignOut={() => void flow.signOutAtAsk()} />
      ) : null}
```

In the root `MacSettings`, beside the update-menu effect:

```tsx
  // Spec 2026-10-10 §5.1: the menu and the compact page hand Settings a flow request.
  const [flowRequest, setFlowRequest] = useState(0)
  useEffect(() => connectSignOutFlowMenu(listen, () => command<boolean>('consume_sign_out_flow_request'), () => {
    setTab('account')
    setFlowRequest((n) => n + 1)
  }), [])
```

and renders `<AccountTab flowRequest={flowRequest} />`.

- [ ] **Step 4: Run, expect PASS.** `bun test tests/signOutFlow.test.ts`: `7 pass`; `bun test tests/macSettingsTabs.test.tsx`: its baseline count plus 2, `0 fail`.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. `signOutAtAsk` passes the first snapshot's `askId` (keep it in a field at the first count): F1 red.
  2. On `unsent_changes:` from `clear_session`, call `count()` and then `sendStart` again (drop `!this.sent`): F10 red.
  3. Treat any `ask: null` as all sent (drop the `closed` check): F11 red (`clearSession` called).
  4. Ignore the generation in `count()`: F11 red (a late count answer starts a phase).
  5. Toast a late `clear_session` error (drop the generation check in `signOut`): F11 red.
  6. Acknowledge before `report`: F12 red on the order.
  7. Consume only on the event (drop `void take()`): F9 red.
  8. `SendingSheet` closes with Cancel: the Settings F3 test red.

- [ ] **Step 6: The frontend gate** with `<N>` = 17; expected `B0 + 4 + 9 pass` (Task 16's 4, this task's 7 and 2), `0 fail`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src/signOutFlow.ts src/desktopApi.ts src/MacSettings.tsx tests/signOutFlow.test.ts tests/macSettingsTabs.test.tsx
git commit -m "feat(macos): Settings sends, asks and keeps before a sign-out (1887)" -- src/signOutFlow.ts src/desktopApi.ts src/MacSettings.tsx tests/signOutFlow.test.ts tests/macSettingsTabs.test.tsx
```

### Task 18: The compact page, the switch step, `forceReauth`

**Files:**
- Modify: `src/pages/Account.tsx:148-166` (`signOut`)
- Modify: `src/onboardingSignIn.ts:40-92` (`SignInSettled` and `settledFrom` carry `unsent`)
- Modify: `src/Onboarding.tsx`: `afterSignIn` (`:148-155`), the step's props (`:193-200`), `AccountSwitchStep` (`:468-503`)
- Modify: `src/accountSwitchCopy.ts:1-6` (its header comment, spec §14), and `src/desktopApi.ts:1047` (the `account_mismatch` type)
- Test: `tests/signOutEntryPoints.test.tsx` (create); `tests/reauthInPlace.test.tsx` (its switch-step stubs learn `unsent`)

**Interfaces:**
- Consumes: `clearSession({ forgetEmail, keepUnsent })` (Task 17), `askLines`, `stripCode`, `A0S_TITLE`, `A1S_LINE`, `AskCounts`, `SignOutCount` (Task 16), commands `request_sign_out_flow`, `sign_out_count`, `sign_out_result_ack` (Tasks 11-13).
- Produces: `SignInSettled`'s `account_mismatch` case is `{ kind: 'account_mismatch'; pendingChanges: number; unsent: AskCounts | null }`; `AccountSwitchStep({ pendingChanges, unsent, onSwitched, onCancel })`.

The browser handoff's `account_mismatch` phase is read only by `WindowsFirstRun.tsx` (`@184:src/WindowsFirstRun.tsx:353`), which never runs on a Mac. On macOS the switch step is reached through the sign-in commands' `LoginOutcome`, so no other reader changes.

- [ ] **Step 1: Write the failing tests** in `tests/signOutEntryPoints.test.tsx`:

```tsx
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as accountSession from '../src/accountSession'
import * as desktopApi from '../src/desktopApi'
import * as diagnosticsCopy from '../src/diagnosticsCopy'
import * as planPresentation from '../src/planPresentation'
import * as copy from '../src/signOutCopy'
import * as switchCopy from '../src/accountSwitchCopy'
import { settledFrom } from '../src/onboardingSignIn'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const unsent = (over: Partial<copy.AskCounts> = {}): copy.AskCounts => ({ askId: 'switch-1', files: 1, copiedFiles: 1, others: 1, missingFiles: 0, earlierFolders: [], ...over })

function switchStep(props: { pendingChanges: number; unsent: copy.AskCounts | null }, backend: Record<string, (args: any) => unknown>) {
  const m = mount('Onboarding.tsx', 'AccountSwitchStep', {
    backend: { sign_out_result_ack: () => null, ...backend },
    expand: true,
    props: { ...props, onSwitched: () => {}, onCancel: () => {} },
    bindings: {
      clearSession: desktopApi.clearSession, command: desktopApi.command, commandUnavailableLabel: desktopApi.commandUnavailableLabel,
      Card: ({ title, copy: body, children }: any) => createElement('section', null, createElement('h3', null, title), createElement('p', null, body), children),
      ...switchCopy, ...copy,
    },
  })
  mounted.push(m)
  return m
}

describe('entry points (spec 2026-10-10 §4)', () => {
  /** F8. */
  test('the switch step renders the snapshot, keeps with its askId, and asks again on a refusal', async () => {
    let refuse = true
    const m = switchStep({ pendingChanges: 2, unsent: unsent() }, {
      clear_session: () => { if (refuse) { refuse = false; throw new Error('unsent_changes: Some changes…') } return { warning: null } },
      sign_out_count: () => ({ ...unsent({ askId: 'switch-2', files: 2, copiedFiles: 2 }), canSend: false, notSending: null }),
    })
    await m.flush()
    const text = textOf(m.tree())
    for (const line of [copy.A0S_TITLE, copy.A1S_LINE, copy.askLines(unsent(), null, true).paragraphs[0], copy.askLines(unsent(), null, true).paragraphs[1]]) {
      expect(text).toContain(line)
    }
    expect(text).not.toContain('removes it')
    expect(text).not.toContain('removes them')
    await m.click(copy.ASK_SWITCH)
    expect(m.calls.find((c) => c.name === 'clear_session')?.args).toEqual({ forgetEmail: true, keepUnsent: { askId: 'switch-1' } })
    expect(m.toasts).toEqual([])
    expect(textOf(m.tree())).toContain('2 files changed in Finder while that account was signed in')
    await m.click(copy.ASK_SWITCH)
    expect(m.calls.filter((c) => c.name === 'clear_session').at(-1)?.args).toEqual({ forgetEmail: true, keepUnsent: { askId: 'switch-2' } })
  })

  test('the switch step with nothing waiting is spec A\'s, and with no snapshot (Linux) it keeps spec A\'s count', async () => {
    const empty = switchStep({ pendingChanges: 0, unsent: unsent({ files: 0, copiedFiles: 0, others: 0 }) }, {})
    await empty.flush()
    expect(textOf(empty.tree())).toContain(switchCopy.ACCOUNT_SWITCH_TITLE)
    expect(textOf(empty.tree())).toContain(switchCopy.accountSwitchBody(0))
    const linux = switchStep({ pendingChanges: 2, unsent: null }, { clear_session: () => ({ warning: null }) })
    await linux.flush()
    expect(textOf(linux.tree())).toContain(switchCopy.accountSwitchBody(2))
    await linux.click(switchCopy.ACCOUNT_SWITCH_CONFIRM)
    expect(linux.calls.find((c) => c.name === 'clear_session')?.args).toEqual({ forgetEmail: true })
  })

  test('another error keeps today\'s toast, then acknowledges it once (F12)', async () => {
    const m = switchStep({ pendingChanges: 1, unsent: unsent() }, { clear_session: () => { throw new Error('Sign-out paused: x') } })
    await m.flush()
    await m.click(copy.ASK_SWITCH)
    await m.flush()
    expect(m.toasts.map((t) => t.title)).toEqual([switchCopy.ACCOUNT_SWITCH_FAILED])
    expect(m.calls.filter((c) => c.name === 'sign_out_result_ack')).toHaveLength(1)
  })

  test('settledFrom carries the snapshot, and reads a missing one as none', () => {
    const base = { requires_2fa: false, reauthenticated: false, vault_unlocked: false, key_replaced: false }
    expect(settledFrom({ ...base, account_mismatch: { pending_changes: 1, unsent: unsent() } } as any)).toEqual({ kind: 'account_mismatch', pendingChanges: 1, unsent: unsent() })
    expect(settledFrom({ ...base, account_mismatch: { pending_changes: 2 } } as any)).toEqual({ kind: 'account_mismatch', pendingChanges: 2, unsent: null })
  })

  /** F4. */
  test('the compact page opens the flow on a refusal, never a toast, and never keeps by itself', async () => {
    // The page's harness, as `tests/accountSessionWarnings.test.tsx` mounts it (`openAccountPage`).
    const m = mount('pages/Account.tsx', 'Account', {
      backend: {
        sync_status: () => ({ logged_in: true, vault_unlocked: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
        autostart_enabled: () => false,
        account_email: () => 'sam@beebeeb.io',
        account_subscription: () => { throw new Error('not in this test') },
        clear_session: () => { throw new Error('unsent_changes: Some changes…') },
        request_sign_out_flow: () => null,
      },
      bindings: { ...accountSession, ...desktopApi, ...diagnosticsCopy, ...planPresentation, ...copy, WEB_APP_URL: 'https://app.beebeeb.io', useRegionLabel: () => 'Stored in the EU' },
    })
    mounted.push(m)
    for (let i = 0; i < 4; i++) await m.flush()
    await m.click('Sign out')
    expect(m.calls.map((c) => c.name)).toContain('request_sign_out_flow')
    expect(m.toasts).toEqual([])
    expect(m.calls.find((c) => c.name === 'clear_session')?.args?.keepUnsent).toBeUndefined()
  })

  /** F12, the compact page: a failure is shown, then acknowledged once. */
  test('the compact page acknowledges a failure it showed', async () => {
    const m = mount('pages/Account.tsx', 'Account', {
      backend: {
        sync_status: () => ({ logged_in: true, vault_unlocked: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
        autostart_enabled: () => false,
        account_email: () => 'sam@beebeeb.io',
        account_subscription: () => { throw new Error('not in this test') },
        clear_session: () => { throw new Error('Sign-out paused: x') },
        sign_out_result_ack: () => null,
      },
      bindings: { ...accountSession, ...desktopApi, ...diagnosticsCopy, ...planPresentation, ...copy, WEB_APP_URL: 'https://app.beebeeb.io', useRegionLabel: () => 'Stored in the EU' },
    })
    mounted.push(m)
    for (let i = 0; i < 4; i++) await m.flush()
    await m.click('Sign out')
    for (let i = 0; i < 4; i++) await m.flush()
    expect(m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
    const names = m.calls.map((c) => c.name)
    expect(names.filter((name) => name === 'sign_out_result_ack')).toHaveLength(1)
    expect(names.indexOf('sign_out_result_ack')).toBeGreaterThan(names.indexOf('clear_session'))
  })

  test('on macOS "Sign in again" never clears the session', async () => {
    const calls: string[] = []
    const result = await desktopApi.forceReauth({
      platform: async () => ({ ok: true, value: 'macos' }),
      clearSession: async () => { calls.push('clear'); return { ok: true, value: { warning: null } } },
      openOnboardingWindow: async () => ({ ok: true, value: undefined }),
      openReauthWindow: async () => { calls.push('reauth'); return { ok: true, value: undefined } },
    })
    expect(result.ok).toBe(true)
    expect(calls).toEqual(['reauth'])
  })
})
```

- [ ] **Step 2: Run, expect FAIL.** `bun test tests/signOutEntryPoints.test.tsx > $EVID/t18-red.log 2>&1`. Expected: the switch step renders `accountSwitchBody(2)`, `settledFrom` has no `unsent`, the compact page toasts.

- [ ] **Step 3: Implement.** `onboardingSignIn.ts`: the `account_mismatch` case gains `unsent: AskCounts | null`, and `settledFrom` reads it:

```ts
/** The switch's snapshot (spec 2026-10-10 W11), or null when absent or not whole (Linux sends none). */
function unsentFrom(raw: unknown): AskCounts | null {
  if (raw === null || typeof raw !== 'object') return null
  const u = raw as Record<string, unknown>
  const whole = (n: unknown) => typeof n === 'number' && Number.isSafeInteger(n) && n >= 0
  if (typeof u.askId !== 'string' || ![u.files, u.copiedFiles, u.others, u.missingFiles].every(whole)) return null
  if (!Array.isArray(u.earlierFolders) || !u.earlierFolders.every((p) => typeof p === 'string')) return null
  return { askId: u.askId, files: u.files as number, copiedFiles: u.copiedFiles as number, others: u.others as number, missingFiles: u.missingFiles as number, earlierFolders: u.earlierFolders as string[] }
}
```

and `return { kind: 'account_mismatch', pendingChanges: pending, unsent: unsentFrom((account_mismatch as Record<string, unknown>).unsent) }`. `desktopApi.ts:1047`: `account_mismatch: { pending_changes: number; unsent?: AskCounts } | null`. `Onboarding.tsx`: `afterSignIn` keeps `settled.unsent` in state beside `pendingSwitch` (`const [switchUnsent, setSwitchUnsent] = useState<AskCounts | null>(null)`) and passes `unsent={switchUnsent}`. `AccountSwitchStep`:

```tsx
function AccountSwitchStep({ pendingChanges, unsent, onSwitched, onCancel }: { pendingChanges: number; unsent: AskCounts | null; onSwitched: () => void; onCancel: () => void }) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)
  const [ask, setAsk] = useState<AskCounts | null>(unsent)
  const cancelRef = useRef<HTMLButtonElement | null>(null)
  useEffect(() => {
    cancelRef.current?.focus()
  }, [])
  const waiting = ask !== null && (ask.files > 0 || ask.others > 0)
  const switchAccount = async () => {
    setBusy(true)
    const result = await clearSession({ forgetEmail: true, keepUnsent: ask ? { askId: ask.askId } : undefined })
    setBusy(false)
    if (!result.ok) {
      // W11: a refusal re-renders the ask from a fresh count, with no toast.
      if (result.reason.startsWith('unsent_changes: ')) {
        const fresh = await command<SignOutCount>('sign_out_count')
        if (fresh.ok) {
          setAsk(fresh.value)
          return
        }
        showToast({ variant: 'error', title: ACCOUNT_SWITCH_FAILED, message: stripCode(fresh.reason) })
        return
      }
      showToast({ variant: 'error', title: ACCOUNT_SWITCH_FAILED, message: result.unsupported ? commandUnavailableLabel('clear_session') : stripCode(result.reason) })
      void command<void>('sign_out_result_ack')
      return
    }
    if (result.value.warning) {
      showToast({ variant: 'info', message: result.value.warning.sentence, durationMs: null })
      void command<void>('sign_out_result_ack')
    }
    onSwitched()
  }
  const lines = waiting && ask ? askLines(ask, null, true) : null
  return (
    <Card title={waiting ? A0S_TITLE : ACCOUNT_SWITCH_TITLE} copy={waiting ? A1S_LINE : accountSwitchBody(ask ? 0 : pendingChanges)}>
      {lines ? lines.paragraphs.slice(0, lines.foldersAfter).map((line) => <p key={line} className="page-copy">{line}</p>) : null}
      {lines?.foldersLine ? <p className="page-copy">{lines.foldersLine}</p> : null}
      {lines ? lines.folders.map((folder) => <p key={folder} className="page-copy" style={{ fontFamily: 'var(--font-mono)', overflowWrap: 'anywhere' }}>{folder}</p>) : null}
      {lines ? lines.paragraphs.slice(lines.foldersAfter).map((line) => <p key={line} className="page-copy">{line}</p>) : null}
      <div className="button-row" style={{ marginTop: 16 }}>
        <button ref={cancelRef} className="button" onClick={onCancel} disabled={busy}>
          {ACCOUNT_SWITCH_CANCEL}
        </button>
        <button className="button danger filled" onClick={() => void switchAccount()} disabled={busy}>
          {ACCOUNT_SWITCH_CONFIRM}
        </button>
      </div>
    </Card>
  )
}
```

`pages/Account.tsx` `signOut`:

```tsx
    if (!result.ok) {
      // Spec 2026-10-10 §4 row 2: a waiting change opens the flow in Settings › Account, never a toast here.
      if (result.reason.startsWith('unsent_changes: ')) {
        await command<void>('request_sign_out_flow')
        setStatus(await loadSyncStatus())
        return
      }
      showToast({
        variant: 'error',
        title: 'Couldn’t sign out',
        message: result.unsupported ? commandUnavailableLabel('clear_session') : stripCode(result.reason),
      })
      void command<void>('sign_out_result_ack')
      setStatus(await loadSyncStatus())
      return
    }
    holdSignOutWarning(result.value.warning?.sentence ?? null)
    if (result.value.warning) void command<void>('sign_out_result_ack')
```

`accountSwitchCopy.ts`'s header: replace "Honest about the loss: the changes that have not uploaded are removed." with "Honest about the loss: on Windows and Linux the changes that have not uploaded are removed; on macOS the step is the sign-out ask of spec 2026-10-10 (A0s-A4s), and they are kept in a folder first." `tests/reauthInPlace.test.tsx`'s `AccountSwitchStep` stub and its `account_mismatch` expectations gain `unsent: null`.

- [ ] **Step 4: Run, expect PASS.** `bun test tests/signOutEntryPoints.test.tsx`: `7 pass`. `bun test tests/reauthInPlace.test.tsx tests/forceReauth.test.ts`: their baseline counts.

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Drop `keepUnsent` from the step's call: F8 red on the args.
  2. Show the refusal as the "Couldn’t sign out" toast: F8 red on `toasts`.
  3. Render `accountSwitchBody(pendingChanges)` whenever `unsent` is set: F8 red on "removes them".
  4. Read a missing snapshot as nothing waiting (`unsent ?? emptyAsk`): the Linux case red.
  5. The compact page toasts `unsent_changes:`: F4 red.
  6. Acknowledge before the toast, or never: the F12 case red.

- [ ] **Step 6: The frontend gate** with `<N>` = 18; expected `B0 + 4 + 9 + 7 pass`, `0 fail`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src/pages/Account.tsx src/onboardingSignIn.ts src/Onboarding.tsx src/accountSwitchCopy.ts src/desktopApi.ts tests/signOutEntryPoints.test.tsx tests/reauthInPlace.test.tsx
git commit -m "feat(macos): the compact page and the account switch reach the sign-out ask (1887)" -- src/pages/Account.tsx src/onboardingSignIn.ts src/Onboarding.tsx src/accountSwitchCopy.ts src/desktopApi.ts tests/signOutEntryPoints.test.tsx tests/reauthInPlace.test.tsx
```

### Task 19: Settings › Sync: our row

**Files:**
- Modify: `src/MacSettings.tsx`: `SyncTab` (`@184:src/MacSettings.tsx:547-740`) reads, shows and dismisses our folder above 1882's row
- Test: `tests/keptUnsentRow.test.tsx` (create)

**Interfaces:**
- Consumes: commands `kept_unsent_folder`, `dismiss_kept_unsent_folder`, `show_kept_unsent_folder` (Task 10); `subscribeKeptFolderChanged` (`src/finderSetup.ts`); `K2_ROW`, `K2_SHOW`, `K2_DISMISS`, `K3_TOAST` (Task 16); `KEPT_FOLDER_ROW_SENTENCE`, `keptFolderAfterDismiss` (`src/macSettingsModel.ts`).
- Produces: nothing new for other tasks.

- [ ] **Step 1: Write the failing test** in `tests/keptUnsentRow.test.tsx`, mounting `SyncTab` the way `tests/macSettingsTabs.test.tsx`'s `open('SyncTab', …)` does (copy its `baseBindings`, `backend()`, `finderBus` and the `useFinderSetup` hook module):

```tsx
  /** F5. */
  test('both rows, ours first, each with its own Dismiss; ours has Show in Finder; neither says "your"', async () => {
    const m = open('SyncTab', backend({
      kept_unsent_folder: () => '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12',
      kept_unsynced_folder: () => '/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 14.05)',
      show_kept_unsent_folder: () => { throw new Error(copy.K3_TOAST) },
      dismiss_kept_unsent_folder: () => ({ cleared: true, current: null }),
    }))
    await settle(m)
    const text = visibleText(m)
    expect(text.indexOf(copy.K2_ROW)).toBeGreaterThanOrEqual(0)
    expect(text.indexOf(copy.K2_ROW)).toBeLessThan(text.indexOf(KEPT_FOLDER_ROW_SENTENCE))
    expect(text).not.toMatch(/\byour\b/i)
    expect(buttons(m).filter((b) => b === copy.K2_DISMISS)).toHaveLength(2)
    expect(buttons(m)).toContain(copy.K2_SHOW)
    await press(m, copy.K2_SHOW)
    expect(m.toasts.map((t) => t.message)).toEqual([copy.K3_TOAST])
    expect(visibleText(m)).toContain(copy.K2_ROW) // the row stays until Dismiss
    await pressFirst(m, copy.K2_DISMISS)
    expect(m.calls.find((c) => c.name === 'dismiss_kept_unsent_folder')?.args).toEqual({ path: '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12' })
    expect(visibleText(m)).not.toContain(copy.K2_ROW)
    expect(visibleText(m)).toContain(KEPT_FOLDER_ROW_SENTENCE)
  })
```

(`pressFirst` presses the first button with that label: ours, because ours renders first. Add it beside `press` if the file has none.)

- [ ] **Step 2: Run, expect FAIL.** `bun test tests/keptUnsentRow.test.tsx`. Expected: `K2_ROW` not found.

- [ ] **Step 3: Implement.** In `SyncTab`, beside `kept` and `loadKept`:

```tsx
  // Spec 2026-10-10 §9: the folder a sign-out copied unsent changes into. Our row comes first; 1882's follows.
  const [keptUnsent, setKeptUnsent] = useState<string | null>(null)
  const loadKeptUnsent = useCallback(async () => {
    const result = await command<string | null>('kept_unsent_folder')
    setKeptUnsent(result.ok ? result.value : null)
  }, [])
  useEffect(
    () => subscribeKeptFolderChanged(() => void loadKeptUnsent(), { onSubscribed: () => void loadKeptUnsent() }),
    [loadKeptUnsent],
  )
  const showKeptUnsent = async () => {
    if (keptUnsent === null) return
    const result = await command<void>('show_kept_unsent_folder', { path: keptUnsent })
    if (!result.ok) showToast({ variant: 'error', message: K3_TOAST })
  }
  const dismissKeptUnsent = async () => {
    if (keptUnsent === null) return
    const result = await command<DismissKeptFolderResult>('dismiss_kept_unsent_folder', { path: keptUnsent })
    if (!result.ok) {
      showToast({ variant: 'error', title: 'Couldn’t dismiss that note', message: result.unsupported ? commandUnavailableLabel('dismiss_kept_unsent_folder') : result.reason })
      return
    }
    setKeptUnsent(keptFolderAfterDismiss(result.value))
  }
```

and, right before 1882's `{kept !== null ? (…)}` row:

```tsx
        {keptUnsent !== null ? (
          <Note
            kind="status"
            reason={keptUnsent}
            reasonWraps
            actions={
              <>
                <Btn onClick={() => void showKeptUnsent()}>{K2_SHOW}</Btn>
                <Btn onClick={() => void dismissKeptUnsent()}>{K2_DISMISS}</Btn>
              </>
            }
          >
            {K2_ROW}
          </Note>
        ) : null}
```

- [ ] **Step 4: Run, expect PASS.** `bun test tests/keptUnsentRow.test.tsx`: `1 pass`. `bun test tests/macSettingsTabs.test.tsx tests/finderPreservedFiles.test.ts`: baseline counts (1882's row unchanged).

- [ ] **Step 5: Mutations, each RED then reverted.**
  1. Render one row (drop ours): F5 red.
  2. Put ours after 1882's: F5 red on the order.
  3. Remove the row on a failed "Show in Finder": F5 red on "the row stays".

- [ ] **Step 6: The frontend gate** with `<N>` = 19; expected `B0 + 21 pass`, `0 fail`.

- [ ] **Step 7: Commit.**

```bash
cd $WT && git add -- src/MacSettings.tsx tests/keptUnsentRow.test.tsx
git commit -m "feat(macos): Settings › Sync shows the kept folder, with Show in Finder and Dismiss (1887)" -- src/MacSettings.tsx tests/keptUnsentRow.test.tsx
```

### Task 20: Docs that change with the code, and the branch gate

**Files:**
- Modify: `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` (where the plan tests differently from §13, below; and the Q1/Q2 rulings as given)
- Modify: `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md:137` (1882 §3)
- Modify: `docs/specs/2026-10-09-macos-finder-write-versions.md:373` (§5.4 row 15) and `:1141` (§10.5)
- Modify: `CLAUDE.md` ("Current macOS integration state")
- Modify: `docs/IPC_PROTOCOL.md:767` (the `accepts_closed` reason; under Q1 option A only)
- Workspace (lead): `.claude/tasks/in-development/1887-…md` Notes, "For the release notes"

**Interfaces:** none.

- [ ] **Step 1: Amend the spec where this plan tests differently**, struck and signed "— lead, <date> (Task 20)":
  - U17: "against a stub server" becomes the margin's SQL test (`the_send_phase_makes_live_changes_due_with_a_margin`) plus the backoff arithmetic on the real `next_attempt_count` and `retry_delay_seconds` (`a_phase_never_parks_what_it_made_due`). The landing within the phase is D1's.
  - U35: "counted through test hooks" becomes the third arm's pure gate (`phase_step_due`, at most 61 runs in 60 s) plus a source pin on the arm's body. The runner loop needs a live engine; D7 and D10 drive the arm on a device.
  - U34 (a)-(h) run on `flow_window_closed` and `SendPhase`, the pure flow state, as the spec allows ("On the pure flow state").
  - The send phase's "Checked at least once a second, and on every landing" is checked once a second; a landing shows in the next second's count.
  - Q1 and Q2: the lead's answers, verbatim, with their rulings' names; under Q1 option B, W1 gains the sentence of §15 question 3, option B; under Q2 option B, A5 `time_limit`'s second sentence is the ruled one.

- [ ] **Step 2: The other docs (spec §14).**
  - 1882 §3, under "A limit this fix does not change (sign-out only).": "Closed by `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md` (task 1887): a sign-out by choice now copies every queued Finder save into its own kept folder before the queue is purged, and asks first. — lead, <date> (1887)".
  - 1873 §5.4 row 15's last cell: "~~Queued saves are purged by design (§10.5)~~ Queued saves are kept in a folder on this Mac before the purge, and the person is asked first (spec 2026-10-10, task 1887)". §10.5's m-12 paragraph gets the same sentence beneath it.
  - `CLAUDE.md`, after the 1882 paragraph in "Current macOS integration state": "**A sign-out keeps waiting Finder saves (task 1887).** On macOS `clear_session` gates on an ask snapshot (`sign_out_count`), and refuses with `unsent_changes:` while changes wait and the person was not asked. Settings › Account sends first (`sign_out_send_start`, at most 60 s, Rust-owned, a third `select!` arm in the runner), then asks. On "Sign out" the keep step closes Finder accepts, copies every waiting save into `<container>/Data/Documents/Changes not sent/<date at time>/` with `clonefile` (never overwriting), and records it durably in `desktop.toml` (`kept_unsent_folder`, `kept_unsent_unfinished`) before the purge, whose guard refuses before any deletion unless every waiting write was kept. One alert names our folder and macOS's; Settings › Sync shows our row with Show in Finder and Dismiss. Closing Settings ends the flow; a late result is shown in the native dialog. Windows and Linux are unchanged. Spec: `docs/specs/2026-10-10-macos-sign-out-waiting-writes.md`."
  - `docs/IPC_PROTOCOL.md:767`, the refusal reasons: add "`accepts_closed` (a sign-out closed Finder accepts after its engine stop; nothing queued, the reply is `WriteRetryLater`)".
  - The task file's Notes (workspace, lead): a paragraph "For the release notes" that the release author pastes into `RELEASE_NOTES.md` for the version that carries 1873 and 1887, under "What's New": "[macOS] Signing out no longer loses a save that is still waiting to upload. Beebeeb first tries to send waiting changes, for at most a minute, then asks before signing out, with the real count. Every save it could not send is copied to a folder on this Mac (in the app's container, under Documents › Changes not sent), shown in an alert and in Settings › Sync." The counted test results are the release's own, at release time (`docs/RELEASING.md`). `RELEASE_NOTES.md` is not edited here: it names 0.8.11.

- [ ] **Step 3: The branch gate, in a fresh tree at the branch head** (the lane's tree has built every task; a fresh one proves the branch alone):

```bash
git -C $WT worktree add --detach ~/code/bb-worktrees/desktop-1887-gate HEAD
G=~/code/bb-worktrees/desktop-1887-gate
cd $G/src-tauri && cargo fmt --check > $EVID/t20-fmt.log 2>&1; echo "fmt rc=$?"
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t20-check.log 2>&1; echo "check rc=$?"
$LOCK cargo-build -- cargo test --locked > $EVID/t20-cargo-test.log 2>&1; echo $? > $EVID/t20-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py --self-test; python3 ../scripts/assert-cargo-test-counts.py $EVID/t20-cargo-test.log --cargo-exit-code "$(cat $EVID/t20-cargo-test.exit)"
grep "test result:" $EVID/t20-cargo-test.log      # lib: ok. L0 + 85 passed; 0 failed; I0 ignored; keychain, wsw, wsc at baseline
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t20-clippy.log 2>&1; grep -c "^warning" $EVID/t20-clippy.log   # ≤ C0
cd $G && bun install --frozen-lockfile > /dev/null && bun test > $EVID/t20-bun.log 2>&1; grep -E "^ *[0-9]+ (pass|fail)" $EVID/t20-bun.log   # B0 + 21 pass, 0 fail
bunx tsc --noEmit -p . > $EVID/t20-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/t20-lint.log 2>&1; echo "lint rc=$?"
bash scripts/test-ipc-framing.sh > $EVID/t20-swift.log 2>&1; tail -1 $EVID/t20-swift.log   # 102 passed, 0 failed: no Swift changed
git diff --stat 184ac962 -- src-tauri/Cargo.lock bun.lock BeebeebFileProvider   # empty
```

  Every number goes into the task file's Notes. Under Q1 option B, the lib count is `L0 + 80` (Task 7's five tests are not there); under Q2 option B, `L0 + 84` (U46 is not there); under both, `L0 + 79`.

- [ ] **Step 4: The other platforms (U32's other half).** The lead pushes the branch and opens the pull request. The public CI's `rust-linux` job and the Windows leg of `rust-check` run `cargo test --locked` there. Evidence: each job's log with `sign_out_unchanged_elsewhere_tests::the_unsent_gate_is_macos_only` and its binary's `test result:` line, as checked by `scripts/assert-cargo-test-counts.py`. If CI cannot run, the Linux half runs on the Linux build host (workspace memory: the Legion host) and the Windows half stays open, named.

- [ ] **Step 5: Commit the docs.**

```bash
cd $WT && git add -- docs/specs/2026-10-10-macos-sign-out-waiting-writes.md docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md docs/specs/2026-10-09-macos-finder-write-versions.md CLAUDE.md docs/IPC_PROTOCOL.md
git commit -m "docs: a sign-out keeps waiting Finder saves; the specs and the repo guide say so (1887)" -- docs/specs/2026-10-10-macos-sign-out-waiting-writes.md docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md docs/specs/2026-10-09-macos-finder-write-versions.md CLAUDE.md docs/IPC_PROTOCOL.md
```

  The gate tree is removed only by the lead, after the pull request merges (`git -C $WT worktree remove ~/code/bb-worktrees/desktop-1887-gate`).

### Task 21: The device rung (lead only)

**Lead only, on the lead's Mac, by the lead.** No lane runs, quits or drives the app, Finder, System Settings, CloudStorage or the containers. Everything runs against the local API with test accounts, under a macOS user account used only for QA (spec §13.4, review M6). Nothing touches production. The steps, their setup and their pass lines are the spec's §13.4, D1-D11 and D10b; this task is the order and the recipe. Evidence goes to `$EVID` as `D*-…`.

**Gate:** Task 20's branch gate passed, and the lead has answered Q1 and Q2.

- [ ] **Step 1: Build the signed QA app from the gate tree** (1873's recipe, `docs/superpowers/plans/2026-10-10-macos-finder-write-versions.md`, Task 13 step 2):

```bash
security find-identity -v -p codesigning | grep "Apple Development"
export APPLE_SIGNING_IDENTITY="<the Apple Development identity printed above>"
export MACOS_APP_PROVISION_PROFILE=~/Downloads/beebeeb-qa-app.provisionprofile
export MACOS_FILE_PROVIDER_PROVISION_PROFILE=~/Downloads/beebeeb-qa-ext.provisionprofile
G=~/code/bb-worktrees/desktop-1887-gate
cd $G && bun install --frozen-lockfile
$LOCK cargo-build -- bunx tauri build --debug --target aarch64-apple-darwin --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked > $EVID/D0-qa-build.log 2>&1; echo "rc=$?"
APPQA=$G/src-tauri/target/aarch64-apple-darwin/debug/bundle/macos/Beebeeb.app
codesign --verify --deep --strict "$APPQA" && echo "verify OK"
codesign -d --entitlements - "$APPQA" 2>&1 | grep -c app-sandbox          # 1
/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APPQA/Contents/Info.plist"
```

- [ ] **Step 2: Setup (spec §13.4 "Setup"),** under the QA-only macOS user: the local API started by the lead with its PID recorded (`cd …/repos/server && cargo run -p beebeeb-api --bin beebeeb-api`, then `curl -sf localhost:3001/health`); `ls -la ~/Library/Containers/io.beebeeb.app/Data/` and the Keychain's Beebeeb items recorded (test accounts only); each test file made with `head -c 1048576 /dev/urandom` and its `shasum -a 256` recorded first; the app launched from Terminal for each D step with `NO_COLOR=1 BB_API_BASE=http://localhost:3001 "$APPQA/Contents/MacOS/<executable>" > $EVID/<step>-app-stdout.log 2>&1` (D7 and D10 use `:3002`); `lsof -a -i -p <the app's pid>` before D1, D7, D8, D10 and D10b shows only the local API or the proxy.

- [ ] **Step 3: Run D1-D6, D8, D9, D10b and D11** exactly as §13.4 states each, and capture each pass line's evidence: screenshots, `grep` of the stdout file for `sign-out flow counted`, `sign-out send phase ended`, `kept unsent changes` and `kept folder reveal requested`, `ls -la` of each kept folder, every `shasum -a 256`. D2 also records the two answers about macOS's folder (§13.4, amended): a queued save that is not parked, and a parked one (park it first with a stale-base save: change the file on the web, then save it in Finder before the change reaches this Mac). Under Q2 option A, D2 has no account case; an account refusal is U46's. D11 records the held folder and the held save under `key_replaced` (§13.4, D11).

- [ ] **Step 4: Run D7 and D10** with the throttling proxy on `127.0.0.1:3002` (a scratch script in the lead's scratch directory, never committed, run without sudo, its PID recorded and stopped with `kill <pid>`). The chunk bound and the rate are recorded. If no proxy setup meets D7's bound, D7 and D10 stay open, named, and the lead records a ruling on U16 in the task file (§13.4).

- [ ] **Step 5: Record the result.** In the task file: one line per D step, pass or fail, with its evidence paths. Any fail sends the task back to `in-development/` with the failing step named; nothing in Task 22 runs. If "Show in Finder" fails D2, §7.1 is amended (the button struck with the evidence named) and Task 19's button is removed in a follow-up commit, before Task 22.

### Task 22: Remove the release blocker (lead only, after Task 21 passes)

**Gate:** Task 21 recorded every required D step as passed (D1-D11 and D10b; D7 and D10 either passed or covered by the lead's recorded ruling on U16), the pull request is merged to `main`, and the task file names the merge commit.

**Files:**
- Modify: `RELEASE_BLOCKERS.md` (its one item)

- [ ] **Step 1: A tree of its own on `main` at the merge commit, then read the file.**

```bash
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop fetch origin
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add ~/code/bb-worktrees/desktop-1887-blocker -b chore/1887-release-blocker-resolved origin/main
cd ~/code/bb-worktrees/desktop-1887-blocker
git log --oneline -1                                             # the merge commit the task file names
cat RELEASE_BLOCKERS.md
grep -nE '^[[:space:]]*[-*][[:space:]]' RELEASE_BLOCKERS.md     # one line: the sign-out item
```

- [ ] **Step 2: Remove the item, keep the file.** Delete only the line that starts with `- A sign-out discards Finder saves that are still waiting to upload.`; the header paragraph stays, so the next blocker has a place (spec §14, amended).

- [ ] **Step 3: Prove both checks now pass, and still refuse with an item.**

```bash
grep -nE '^[[:space:]]*[-*][[:space:]]' RELEASE_BLOCKERS.md; echo "items rc=$?"     # prints nothing; rc=1
bash scripts/test-release-macos-local.sh > $EVID/t22-release-shim.log 2>&1; tail -2 $EVID/t22-release-shim.log   # release-macos-local: 410 passed, 0 failed (its own fixture repo; the blocker scenario still refuses)
bash scripts/test-release-macos-local.sh --self-test; echo "self-test rc=$?"
```

- [ ] **Step 4: Commit in that tree and merge through a pull request** (the blocker's removal is its own change, never the code's):

```bash
cd ~/code/bb-worktrees/desktop-1887-blocker
git commit -m "chore: a sign-out keeps waiting Finder saves; remove the release blocker (1887)" -- RELEASE_BLOCKERS.md
```

  The pull request names Task 21's evidence. Only after it merges is 1887 moved to `ready-to-ship/` (deploy-gated: a desktop release) by the lead.

---

## Execution

**Recommended: subagent-driven, one Rust lane then one TypeScript lane** (at most two at once, workspace rule). The Rust tasks consume each other's names (`WaitingChanges` → `KeptSet` → the purge's `kept` → the gate → the phase), so a fresh reviewer per task catches a drifted signature before the next task builds on it, and a mistake that ships here deletes a person's file. Tasks 7 and 11 wait for the lead's answers to Q1 and Q2; every other task can start now. Task 15 is reviewed by the lead before Task 16 starts. Tasks 21 and 22 are the lead's.
