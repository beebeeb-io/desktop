# Desktop First Run: Sign In Only — Implementation Plan (spec C of 3; tasks 1747, 1748)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Mac or PC that has no working session opens one centred window that says "Sign in to Beebeeb" (browser first, password second, account creation on the web), the menu bar shows a crossed b whenever Beebeeb is disconnected, and after sign-in the server's onboarding document decides, through one Rust-owned account state, whether Finder and sync start, the person is asked to choose a plan on beebeeb.io, or the app must be updated.

**Architecture:** A new Rust module `src-tauri/src/account_view/` fetches `GET /api/v1/onboarding`, parses it tolerantly (`doc`), resolves every web link (`links`), formats the Ready notice (`notice`), derives one `AccountView` from the session, the keys, the document and connectivity (`derive`, the §4.2 table), schedules fetches on an injected clock (`policy`), keeps one cached account part per account in `state.db`, and owns the account gate `{ open, epoch }` (`gate`) that every engine start and the Finder reconciler's add read at the point of action. One async task (`driver`) ties these together and emits `account-view-changed`. Two small pure units decide the macOS launch (`launch_kind`, from the open-application Apple event) and the menu-bar icon (`tray_presentation`). The windows only render the view: macOS's `onboarding` window becomes the account window, Windows renders the same states in its existing windows, and Linux gets the shared sign-in changes and nothing else.

**Tech Stack:** Rust 2024 (Tauri 2.11, tokio, serde, chrono, reqwest; no new crate, one new tauri feature `image-ico`), Objective-C (Foundation, compiled by `src-tauri/build.rs`), React 19 + TypeScript (bun test with `tests/fixtures/componentHarness.ts`), eslint. Design mocks in plain HTML.

**Spec:** `docs/specs/2026-10-07-desktop-first-run-sign-in.md` (HEAD `f80922b`, approved by Guus 2026-10-07 in three parts with rulings C1–C7, amended by the lead's rulings in §17). It builds on spec A, `docs/specs/2026-10-06-macos-finder-setup-reconciler.md`, and its plan `docs/superpowers/plans/2026-10-06-macos-finder-setup-reconciler.md`. Executors read the spec and this plan together. Where they disagree, "Spec issues found" below says how, with evidence; nothing there is decided silently.

---

## Spec issues found

Each item is a place where the spec, read literally against the code on spec A's branches (`feat/1834-finder-reconciler`, `feat/1834-finder-reconciler-ui`, read 2026-10-07) or the server's contract (server `d2e7f776`), needs an interpretation. The plan implements the interpretation stated. The lead may reject any one of them on its own; each names the task and test that carry it.

1. **The unknown-step fixture is a pre_account document.** `contracts/onboarding/fixtures/forward_compat.unknown_step.ios.json` has `"stage": "pre_account"` (server `d2e7f776`), and the desktop never reads `steps` from a pre_account document (C-D3). It therefore proves only that unknown steps and unknown fields parse. The C-R5 button's "unknown required step" link rule (§5.1, last row) is tested on an account-stage document derived in the test from `account.needs_plan.web.coupon.json` with one extra required step `future_step` carrying a `fallback.url`. Task 3, `the_step_link_is_the_first_required_step_with_a_checked_url`.
2. **`account_ready` on Windows and Linux must not restart a running engine.** C-R12 says it "calls today's engine start, `start_engine_if_possible`". That function stops and respawns an engine already in the slot (spec A `lib.rs`, the `engine_slot.take()` branch). Two of C-R12's edges reach Ready with an engine running: Session ended → Ready when spec A's auth-expired state clears by itself, and launch on Linux, where the gate never holds. Calling it there would restart sync for no reason. The plan calls `start_engine_if_possible` only when the engine slot is empty (`start_engine_when_none_runs`). Task 11, `account_ready_starts_the_engine_only_when_none_runs`. Review Focus 3.
3. **The gate follows the account, not the key.** C-R10 says the gate is closed "in Checking, C-R3 and C-R5 while the session is valid, and open otherwise", so it reads open in Locked (C-R4). But spec A's unlock paths (`unlock_vault`, `desktop_unlock_with_recovery_phrase`, `apply_session`) install the key and call `start_engine_if_possible` in the same command, before the account-view driver has re-derived anything. A no-plan account with a cached blocking part would then start its engine between the unlock and the view update. The plan computes the gate from the account's condition with the key assumed present: it is closed while the session is valid and the account would be Checking, Update required or No plan, whether or not the key is in memory. In every state where the literal rule and this one differ, no engine can start anyway (no key), so the only behaviour that changes is that gap. Task 5, `the_gate_follows_the_account_not_the_key`. Review Focus 1. **Confirmed by the lead 2026-10-07.**
4. **"Valid session" excludes the startup restore.** The §4.2 table puts Checking's first entry ("a stored token whose startup restore and probe are running") after C-R4 in C-R8's order, and during the restore no key is in memory, so a literal reading derives Locked for every launch. The plan reads "valid session" in C-R4 as "a stored token whose restore has finished and that is not auth-expired", so the restore derives Checking as the table's Checking row says. Task 5, `checking_covers_the_startup_restore`.
5. **A single network failure ends Checking.** §4.1 defines "unavailable" as "that fetch ended without a usable document (§10)". The Offline overlay (C-R7) needs 2 network failures at least 10 s apart. The plan ends Checking on the first concluded fetch of any kind (network failure included) and still shows Offline only after the second failure. Without this, a launch with no connection would sit on the busy screen for at least 10 s. Task 10, `a_first_network_failure_ends_checking_without_showing_offline`. Review Focus 2. **Confirmed by the lead 2026-10-07.**
6. **Any HTTP answer clears Offline.** §10 says Offline "clears on the next success" and also that "offline means network-level only; an HTTP status means document unavailable". The plan treats any completed HTTP exchange (2xx, 401, 429, 4xx, 5xx) as proof that the network works: it clears the Offline overlay and resets the network-failure count. Only a 2xx resets the failure count used for the 10 s retry. Task 6, `an_http_answer_clears_offline_and_only_a_2xx_resets_the_retry`.
7. **A trial with no end date has no notice.** §7.5's trial line needs `account.trial.ends_at`. When neither that date nor server `copy` is present the plan shows no notice (the plan button stays "Manage plan"), because the read-only rule "never invent a date" (OP-3) applies here too. Task 4, `a_trial_without_an_end_date_or_server_copy_has_no_notice`.
8. **"Read-only." alone has no link.** For an unknown `account.state` whose upload denial is not a plan reason, §7.5 gives the line "Read-only." and no link. The plan gives that notice (`read_only_other`) no link, and the Settings plan button then stays as it is. Task 4, `an_unknown_state_denied_for_another_reason_says_read_only_alone_with_no_link`; Task 18.
9. **The cache row is not account data.** Spec A's `ACCOUNT_TABLES` rows count as account data (`has_account_data`), and R10's first-start check reads that count. The cache row is written after a sign-in, before the first engine start, so listing it there would change R10's binding decision on every first sign-in. The plan adds a third list, `ACCOUNT_BOUND_TABLES`: bound to one account, holding no user content, emptied by every reset and purge, never counted. Task 7, `the_cache_row_is_never_account_data`.
10. **Tooltips live in Rust.** §7 says all strings live in `src/accountViewCopy.ts`, but the three tooltips (C-I2) are set by Rust on the tray and no frontend runs while the app sits in the menu bar. They are pinned in `src-tauri/src/tray_presentation.rs` by a Rust test, and the TS copy test pins the same three strings by reading that file (Task 21, the cross-language pins).
11. **"Quit Beebeeb" needs a command.** No command quits the app today (only the native menu's `DesktopMenuAction::Quit`). The plan adds `quit_app` (`app.exit(0)`). Task 11.
12. **C-W7 needs Settings opened on its Account tab.** C-W3 and C-W4 open "the Settings window" for C-R4 with the key in the Keychain. The Unlock button is on the Account tab, and `show_macos_settings_window` opens whatever tab the window last showed. The plan opens it on `?tab=account` (the query `settingsTabFromSearch` reads). Task 13, `the_keychain_unlock_opens_settings_on_the_account_tab`. Review Focus 5. **Confirmed by the lead 2026-10-07.**
13. **The account window cannot unlock from the Keychain by itself.** If the account window is already open when the state becomes C-R4 with the key in the Keychain (C-W6: it re-renders), there is no approved screen for the Keychain unlock inside it. The plan hands over: the window asks Rust to open Settings on the Account tab and hides itself. No new copy. Task 17.
14. **`start_browser_login` returns `LoginOutcome`.** C-S1 makes the browser command return the same shape as `desktop_login`. Windows' first-run screen consumed `Result<(), String>` and the `done` event; it now routes on the returned outcome through `settledFrom` like macOS and Linux. Task 12, Task 16.
15. **The session generation is spec A Task 12's.** C-D5 binds every fetch to a session generation "provided by spec A Task 12; spec C adds it only if absent". Task 12's rulings (item 11) require "one small, documented API (e.g. `session_generation()` plus a check)". Task 12 is not written yet, so the exact name is unknown today. This plan consumes `crate::session_generation() -> u64`. Task 0 records the name Task 12 actually merged and the executor substitutes it everywhere this plan says `session_generation`. If Task 0 finds no generation API at all, Task 0 stops and the lead decides between adding it (C-D5's fallback) and waiting.
16. **Carry-over T11-M5 changes the switch warning's copy.** Making the pending-changes count `null` on an unreadable `state.db` (instead of 0) needs a sentence that does not claim a number. It is new copy, so Task 1 draws it and the lead approves it with the mocks. Proposed: "This Mac is signed in to another Beebeeb account. Beebeeb couldn’t count the changes on this Mac that haven’t uploaded yet. Switching signs that account out of this Mac and removes any that are left." Tasks 12 and 16 implement it only if Task 0 finds M5 still open at HEAD.
17. **The 15-minute re-check starts before the browser opens.** §7.4 says the click "starts the 15-minute re-check (C-D4) whether or not the browser opened". The button calls `account_view_plan_opened` first and opens the link second, so a failed open still polls and still shows the address with "Copy link". Task 17, `the_plan_poll_starts_before_the_browser_opens`.
18. **The variant B icon files live in the workspace repo.** They are committed at `$WS/design/desktop-first-run/tray-icon/` (workspace commit `39494ebcf`): `tray-template-disconnected.png`, `tray-template-disconnected@2x.png` and `tray-template-disconnected.svg`, with the generator scripts and fitted glyph parameters in `source/`. Their SHA-256 sums are recorded in Task 1 and were checked against these copies on 2026-10-07 (3 of 3 match). If a file is missing or differs, Task 1 stops; it never redraws them.
19. **Checking has a 15 s limit** (lead ruling 2026-10-07, plan review I5). The spec ends Checking only on a concluded first fetch, and the startup restore holds the fetch until it reports back, so a restore task that dies would leave the view in Checking for the whole process. The plan ends Checking at the latest 15 s after it began (a launch, or a new session generation): the 5 s probe cap plus margin. That concludes "document unavailable", which fails open like any failed first fetch: Ready with the key in memory, Locked without it (C-R4 still applies), Signed out without a session. A sign-in always leaves Checking, at the latest 15 s after it. Task 6 `checking_gives_up_15s_after_it_began`; Task 10 `a_dead_restore_task_still_ends_checking_after_15s`, `a_sign_in_during_a_stuck_restore_leaves_checking`; Task 11, the restore's drop guard.
20. **The gate is closed for a session generation the account view has not derived** (lead ruling 2026-10-07, plan review I1). C-R10 reads open in Signed out, and spec A's `apply_session` installs a sign-in's keys and starts the engine in the same command, before the account view can derive anything. The gate therefore carries the generation it was derived for and reads closed for every other one, evaluated at the point of action (spec ruling I-4). The engine start a sign-in makes finds it closed; the account view's entry into Ready starts the engine afterwards (C-R12). Linux arms the gate so that it never holds; unit tests that build an `AppState` get an unarmed gate. Task 9 `a_generation_the_driver_has_not_derived_is_closed`, `a_first_sign_in_with_a_blocking_account_starts_no_engine`; Task 10 `the_driver_sets_the_gate_for_its_own_generation`.
21. **A wake needs no separate path.** §10 detects a wake "when the wall clock moves more than 60 s across one 30 s probe tick" and re-probes on it. While offline the only tick is the 30 s re-probe itself, and at that tick the re-probe is due anyway: the timer runs on the monotonic clock, which does not advance while the Mac sleeps, so the tick comes at most 30 s of awake time after the last probe, wake or not. A wake branch can never change what happens, and no test of it could fail its mutation (plan review I8). The plan builds none: the 30 s offline re-probe is the wake re-probe. Task 23 checks it on the Mac (a wake while offline re-probes within 30 s). Task 6, Task 10.

## Global Constraints

- **Environment (every task; a task's steps use these names).**
  ```bash
  WS=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io
  LOCK=$WS/scripts/coord/with-lock.sh                 # every cargo build or test: $LOCK cargo-build -- <cmd>
  EVID=$WS/.claude/tasks/_qa-evidence/1747            # Windows rungs (Task 24): EVID1748=$WS/.claude/tasks/_qa-evidence/1748
  WTR=~/code/bb-worktrees/desktop-1747-r              # Lane R, branch feat/1747-first-run-sign-in
  WTT=~/code/bb-worktrees/desktop-1747-t              # Lane T, branch feat/1747-first-run-sign-in-ui
  WT=$WTR                                             # in Tasks 1–14 (Task 1 writes the icons there); WT=$WTT in Tasks 15–21
  ```
  - **The Rust loop command** ("run with filter X"), from `$WT/src-tauri`:
    ```bash
    cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib <filter> > $EVID/t<N>-<step>.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "test result:|error\[|FAILED|panicked" $EVID/t<N>-<step>.log | head -20
    ```
    `<filter>` is one substring, or `-- <name> <name> …` for several exact names (libtest runs every test that matches any of them). A step that expects N tests compares N with the `test result:` line; fewer means a filter missed.
    Before every Rust commit also run `cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t<N>-check.log 2>&1; echo "rc=$?"` and expect `rc=0`.
  - **The TypeScript loop command**, from `$WT`:
    ```bash
    cd $WT && bun test tests/<file> > $EVID/t<N>-<step>.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)|error:" $EVID/t<N>-<step>.log
    ```
    Before every TS commit also run `bunx tsc --noEmit > $EVID/t<N>-tsc.log 2>&1; echo "rc=$?"` and `bun run lint > $EVID/t<N>-eslint.log 2>&1; echo "rc=$?"` (the repo's `eslint .`), both `rc=0`.
  - `grep` in a guard or a check is always `/usr/bin/grep` (the shell's `grep` is a ugrep function with other exit codes).
  - Task 0 writes `$EVID/t0-record.md`: the facts about spec A as merged that Tasks 9–13 branch on (`SESSION_TRANSITION` on every platform or Windows only; the session generation's name, type and bump sites; the six `spawn_bound_engine` callers; the startup 401's behaviour; T11-M5; the signatures). A step that says "per `t0-record.md`" reads it there.
- **Lane rules (binding on every implementer; spec A's `lane-rules.md`, copied here).**
  - The first action of every task is one Bash block: `cd $WT && git status --short && git log --oneline -1`. Report a blocker within 5 minutes; never sit silent.
  - Work only in your lane's worktree; never in `$WS/repos/desktop` (the primary checkout stays on `main`, clean) and never in the other lane's tree.
  - Never `git add` or commit anything in the workspace repo (`$WS`): evidence and Notes go under `$EVID`, and the lead commits them.
  - No network installs. The only install is Task 0's `bun install --frozen-lockfile` in each lane's tree (lockfile-pinned).
  - If the plan is wrong against the code (a line moved, a signature differs), adapt minimally and say so in the report; if it is wrong in substance, report NEEDS_CONTEXT or BLOCKED with `file:line` evidence.
- **Intermediate states.** Between Task 2 and Task 11 the new Rust items are not called from the app: Task 2's module and Task 9's `hold_for_account` carry `#[allow(dead_code)]`, both removed in Task 11. Clippy is compared with the baseline at Task 14 and Task 22; no task may add a clippy warning in a file this plan creates.
- **Order.** Spec C executes only on a `main` that contains spec A's merged Lane R (`feat/1834-finder-reconciler`) and Lane T (`feat/1834-finder-reconciler-ui`). Task 0 proves it.
- **Design before code.** Task 1's mocks (light and dark) and icon assets are approved by the lead before any code task runs. The copy is verbatim from the spec. Where an approved mock and the spec disagree, the mock wins and the spec is amended in the same change (spec §12).
- **Copy (exact, typographic `’` in code, spec §7):**
  - Sign-in: title `Sign in to Beebeeb`; primary `Sign in with browser`; text link `Use email and password`; footer `New to Beebeeb?` + `Create an account on beebeeb.io`; small `Quit Beebeeb`.
  - Offline: `Can’t reach Beebeeb. Check your connection.` + `Try again`.
  - Session ended (spec A's wording): `Sync is paused until you sign in again.` / `Sign in again`.
  - Update required: `Update Beebeeb` — `This version of Beebeeb is too old to connect. Update to continue.` — `Update now`.
  - No plan: `Choose a plan` — `Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.` — `Choose a plan on beebeeb.io`; after the click `Waiting for your plan. This window updates by itself.`; quiet `Sign out`.
  - Notices: `Free trial until {date}.` / `Read-only since {date}.` or `Read-only.` + `Files are deleted on {date} unless you choose a plan.` or `Choose a plan to upload again.` / `This account is frozen.` / `Payment failed. Update your payment details on beebeeb.io.`; links `Manage plan`, `Choose a plan`, `Contact support`, `Update payment details`.
  - Browser won't open: the address as selectable text + `Copy link`.
  - Tooltips: `Beebeeb: Signed out`, `Beebeeb: Sign in again`, `Beebeeb: Offline`.
- **Dates** are day-month (`18 Oct`), UTC, English month abbreviations, formatted in Rust (`account_view::notice::day_month`), identical to the server's `onboarding::day_month`.
- **Links.** Built-in addresses: `https://app.beebeeb.io/signup` (create account), `https://app.beebeeb.io/billing` (web billing), `https://beebeeb.io/support` (support). A server link counts only if it is `https`, has a host, has no user-info, and the host is `beebeeb.io` or a subdomain; debug builds also accept `http://localhost`. In TypeScript no file but `src/accountLinks.ts` contains those three literals.
- **Schedule (C-D4):** account ttl 60 s, pre_account ttl 300 s, plan poll 3 s for 15 minutes; ttl clamped to 30–900 s, poll to 2–30 s, absent or 0 takes the built-in; first-failure retry 10 s; Offline after 2 network failures at least 10 s apart; 30 s re-probe while offline, which is also the wake re-probe (Spec issue 21); Checking ends at the latest 15 s after it began (Spec issue 19); 429 waits `Retry-After` or ttl, capped at 300 s; macOS launch settle 5 s after the restore probe.
- **Request (C-D2):** `GET {BB_API_BASE}/api/v1/onboarding` with today's `X-Beebeeb-Client: desktop` and `X-Beebeeb-Client-Version`, plus `X-Beebeeb-Onboarding-Schema: 1` and `X-Beebeeb-Client-OS: macos|windows|linux`; Bearer only while signed in.
- **Never logged (C-D6):** the document body never reaches `tracing`, the lifecycle log, "Copy details" or the support bundle. Lifecycle lines are the three closed-vocabulary events of §10, macOS only (Windows and Linux: `tracing` only), with no email, account id or URL.
- **Platforms.** macOS gets the launch policy (C3), the detached menu, the account window and the removals (§11). Windows gets the states, copy and crossed icon in its existing windows, no launch policy and no click change. Linux gets the Rust state, the fetch, the icon and the shared sign-in window changes; its gate never holds and nothing Linux-specific is built.
- **Out of this plan:** the server change (workspace task 1844; the client reads `fallback` when present and uses the built-in addresses otherwise) and macOS autostart registration (workspace task 1845). Spec C's login-launch detection is in.
- **No new dependency.** `bun.lock` is unchanged. `Cargo.lock` adds no `[[package]]` entry (the one feature change is tauri's `image-ico`, Task 14, proved by a before/after count).
- **Brand:** one accent, amber (oklch 0.82 0.17 84), only for primary actions and encryption state. Inter for human text, JetBrains Mono for machine text (the device code, addresses). No emoji. Product copy says "Europe"/"the EU" and never names the hosting provider.
- **Test isolation (binding on every task):**
  - The real Keychain store stays compiled out under `cfg(test)` (spec A `keychain.rs`); tests use only test-scoped service and account names.
  - Config (`desktop.toml`), the finder-writes cache and the hydrate dir resolve to `test_sandbox::dir(..)` in tests (spec A `test_sandbox.rs`); a test that touches `state.db` opens one in a `tempfile::tempdir()`.
  - Full test runs use the scratch-HOME recipe:
    ```bash
    SCRATCH=$(mktemp -d)
    RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/<log> 2>&1; echo "rc=$?"
    rm -rf "$SCRATCH"   # only the directory mktemp made; never $HOME
    ```
  - No test calls the real `lifecycle_log::init`. Driver tests log through a fake port.
  - Every new test is seen to fail first, by a deliberate mutation of the code it covers. The failing lines are pasted into the task's Notes, the mutation is reverted, and the green run is pasted beneath.
- **Truth lines.** Rust: per-binary `test result: ok. N passed; 0 failed` from `cargo test --locked` in `src-tauri/`. TypeScript: the `bun test` count line (`N pass` / `0 fail`). A run that reports 0 tests, or a filter that matched nothing, is a red.
- **Public repo.** `beebeeb-io/desktop` is public. Commit messages and comments stay neutral and describe behaviour; they never describe an open security weakness, a leak or how to trigger one. A bare workspace task number is acceptable (lead ruling R-history, refined); code comments cite the spec instead.
- **Git.** Every task commits with an explicit pathspec (`git commit -m "…" -- <paths>`) and checks `git show --stat HEAD`. Forbidden: `git stash`, `reset`, `checkout --`, `restore`, `clean`, `switch`, rebase (except the lead's Task 21 rebase), push. Lanes never commit `graphify-out/`. Each commit ends with the trailer of the model that wrote it, never the lead's.
- **Process.** One worktree per lane, allocated by the lead. `CARGO_TARGET_DIR` stays unset. Every cargo build or test runs through `$LOCK cargo-build -- …` (rc 75 = busy: wait ~30 s and rerun; never a result). Foreground commands only. No `pkill`/`killall`. Redirect every run to a log under `$EVID`, then grep the log; never `cmd | tail`.
- **Gates (spec §14):** `cargo test --locked` per-binary counts; `bun test` count; `bunx tsc --noEmit`; eslint; no new warnings from `cargo clippy --locked --all-targets` against the `origin/main` baseline Task 0 records.
- **Expected test counts.** `B_lib` is Task 0's `cargo test --lib` count on macOS and `B_bun` its `bun test` pass count. After each task the lib count is `B_lib` plus the running total below, and `bun test` passes `B_bun` plus its running total minus the spec A tests Task 17 deletes (named in its Notes). M5 rows count only if Task 0 found T11-M5 open. A non-macOS lib run lacks the macOS-only tests marked (m). A task reports its own count against this table; a difference is a red until explained.

  | Task | New Rust lib tests | Running (Rust) | New TS tests | Running (TS) |
  |---|---|---|---|---|
  | 2 | 10 | 10 | | |
  | 3 | 7 | 17 | | |
  | 4 | 12 | 29 | | |
  | 5 | 22 | 51 | | |
  | 6 | 15 | 66 | | |
  | 7 | 6 (1 not on Windows) | 72 | | |
  | 8 | 11 | 83 | | |
  | 9 | 17 (gate 6, core 6, macos_ports 1 (m), lib 4) | 100 | | |
  | 10 | 35 (driver 34, lifecycle_log 1) | 135 | | |
  | 11 | 7 | 142 | | |
  | 12 | 3 (+1 M5) | 145 | | |
  | 13 | 19 (launch_kind 3, of them 1 (m); surfaces::policy 11; lib 5) | 164 | | |
  | 14 | 17 (tray_presentation 13, icons 2, lib 2) | 181 | | |
  | 15 | | | 17 | 17 |
  | 16 | | | 18 (+2 M5) | 35 |
  | 17 | | | 20 | 55 |
  | 18 | | | 6 | 61 |
  | 19 | | | 3 | 64 |
  | 20 | | | 3 | 67 |
  | 21 | | | 3 | 70 |

  Running totals leave out the M5 rows. Task 14 also extends two of Task 10's driver tests; it adds none there.

## Review Focus

The five conditions a real person is most likely to meet that the spec implies but its test list (§14) does not exercise. Each line names the test that pins it and the task that owns it.

1. **Unlocking a no-plan account.** The cached account part says `needs_plan`; the person unlocks (Keychain or recovery phrase). They expect "Choose a plan", with no engine started and nothing added to Finder, not a burst of sync between the unlock and the screen. Pinned by `the_gate_follows_the_account_not_the_key` (Task 5) and `an_unlock_with_a_cached_no_plan_part_starts_no_engine` (Task 11). Spec issue 3. The same holds for a first sign-in of a plan-less account, whose engine start comes before the account view can derive anything: the gate is closed for a session generation the view has not derived (`a_first_sign_in_with_a_blocking_account_starts_no_engine`, Task 9; `the_driver_sets_the_gate_for_its_own_generation`, Task 10). Spec issue 20.
2. **Launching with no connection and no cache** (a stored session, first launch after the upgrade, on a train). They expect today's behaviour within one failed fetch: no busy screen held for 10 s, sync starting as before, and no Offline line until the second failure. Pinned by `a_first_network_failure_ends_checking_without_showing_offline` (Task 10). Spec issue 5.
3. **Sync running when an expired session recovers by itself** (spec A clears auth-expired after a later success, with no sign-in). On Windows and Linux they expect sync to carry on, not restart. Pinned by `account_ready_starts_the_engine_only_when_none_runs` (Task 11). Spec issue 2.
4. **Relaunching offline with a no-plan account.** They expect "Choose a plan" to stay, with the offline line and "Try again" instead of the button, and still no Finder entry (C-R13: the cached blocking part keeps its gate). Pinned by `offline_relaunch_with_a_cached_no_plan_part_keeps_choose_a_plan` (Task 5) and `an_offline_relaunch_keeps_the_cached_gate_closed` (Task 10).
5. **Clicking the menu-bar icon while locked with the key in the Keychain.** They expect the Settings window on the tab that has the Unlock button, not the tab they last used. Pinned by `the_keychain_unlock_opens_settings_on_the_account_tab` (Task 13). Spec issue 12.

---

## File map

Paths are relative to the desktop repo root unless they start with `$WS` (the workspace root, `/Users/guuslangelaar/Development/Beebeeb/beebeeb.io`).

| File | Status | Responsibility | Task |
|---|---|---|---|
| `$WS/design/desktop-first-run/*.html` | create | hi-fi mocks, light and dark | 1 |
| `$WS/design/desktop-first-run/tray-icon/` | create | the 22×22 template PNG and the SVG source (reference) | 1 |
| `src-tauri/icons/tray-template-disconnected@2x.png` | create | the crossed b, macOS template, 44×44 | 1 |
| `src-tauri/icons/tray-disconnected.ico` | create | the crossed b, colour, Windows and Linux | 1 |
| `contracts/onboarding/**` | create (vendored) | the server's contract, byte-identical | 2 |
| `src-tauri/src/account_view/mod.rs` | create | module root | 2 (grows to 11) |
| `src-tauri/src/account_view/doc.rs` | create | tolerant v1 parse, `Doc`, `AccountPart` | 2 |
| `src-tauri/src/account_view/links.rs` | create | link checks, built-in addresses, resolution | 3 |
| `src-tauri/src/account_view/notice.rs` | create | Ready notices, `day_month` | 4 |
| `src-tauri/src/account_view/derive.rs` | create | the §4.2 table, C-R8, `AccountView`, gate value | 5 |
| `src-tauri/tests/fixtures/account-view/*.json` | create | the frontend contract examples | 5 |
| `src-tauri/src/account_view/policy.rs` | create | schedule, bounds, offline, settle, the Checking limit | 6 |
| `src-tauri/src/state_db.rs` | modify | the cache table, `ACCOUNT_BOUND_TABLES`, purges | 7 |
| `src-tauri/src/reauth.rs` | modify | `LocalTraces.onboarding_cache` | 7 |
| `src-tauri/src/account_view/fetch.rs` | create | the request, headers, outcome | 8 |
| `src-tauri/src/account_view/gate.rs` | create | `AccountGate`, `GateValue` | 9 |
| `src-tauri/src/finder_setup/{core,error,driver,macos_ports}.rs` | modify | `AccountHold`, `AccountReady`, `ACCOUNT_BLOCKED`, the add's gate check | 9 |
| `src-tauri/src/account_view/driver.rs` | create | the one task, its handle, its ports trait | 10 |
| `src-tauri/src/lifecycle_log.rs` | modify | three new events | 10 |
| `src-tauri/src/account_view/app_ports.rs` | create | the production ports | 11 |
| `src-tauri/src/lib.rs` | modify | gate in engine starts (9); setup, commands, session events (11); browser outcome, M5 (12); launch (13); tray (14) | 9, 11–14 |
| `src-tauri/src/browser_login.rs` | modify | returns `LoginOutcome` (C-S1) | 12 |
| `src-tauri/src/launch_kind.rs`, `src-tauri/macos/LaunchEvent.m`, `src-tauri/build.rs` | create / modify | login-launch detection | 13 |
| `src-tauri/src/surfaces/policy.rs` | modify | `startup_surface` macOS row | 13 |
| `src-tauri/src/tray_presentation.rs` | create | icon, tooltip, click, menu | 14 |
| `src-tauri/Cargo.toml` | modify | tauri `image-ico` | 14 |
| `src/accountView.ts`, `src/accountViewCopy.ts`, `src/accountLinks.ts` | create | the frontend contract, copy, links | 15 |
| `src/browserSignIn.ts`, `src/BrowserSignIn.tsx` | create | the shared browser handoff (moved from Windows) | 16 |
| `src/onboardingSignIn.ts`, `src/accountSwitchCopy.ts` | modify | `fresh.vaultUnlocked`; M5 | 16 |
| `src/accountScreens.tsx` | create | sign-in footer, offline line, update, choose a plan, busy, link fallback | 16, 17 |
| `src/Onboarding.tsx` | modify | shared sign-in (16); macOS account window, removals (17) | 16, 17 |
| `src/WindowsFirstRun.tsx` | modify | mounts the shared sign-in (16); C-R3/C-R5 before Files on demand (19) | 16, 19 |
| `src/design.css` | modify | account window and sign-in classes | 16, 17 |
| `src-tauri/capabilities/account-window.json` | create | `set-title` and `close` for the `onboarding` window | 17 |
| `src/MacSettings.tsx`, `src/macSettingsModel.ts`, `src/pages/Account.tsx`, `src/pages/VersionCenter.tsx`, `src/desktopApi.ts` | modify | notice line, resolved billing link | 18 |
| `src/windows/views/AccountView.tsx`, `src/WindowsApp.tsx` | modify | notice, frozen row, resolved links (18); C-R3/C-R5 in main-app (19) | 18, 19 |
| `src/App.tsx`, `src/compactNavigation.ts` | modify | no Drive status on macOS | 20 |
| `tests/*.test.ts(x)` | create / modify | frontend tests | 15–21 |
| `CLAUDE.md`, `docs/CAPABILITIES.md`, `docs/specs/2026-10-07-desktop-first-run-sign-in.md` | modify | docs and amendment lines | 21 |

## Lanes, branches and order

- **Task 0** (lead): precondition, worktrees, baselines.
- **Task 1** (design lane, one implementer): mocks in the workspace, icon assets on Lane R's branch. **The lead approves the mocks before any of Tasks 2–21 is dispatched.**
- **Lane R (Rust)**: Tasks 2–14, sequential, worktree `~/code/bb-worktrees/desktop-1747-r`, branch `feat/1747-first-run-sign-in`. Each task depends on the types of the one before.
- **Lane T (TypeScript)**: Tasks 15–20, sequential, worktree `~/code/bb-worktrees/desktop-1747-t`, branch `feat/1747-first-run-sign-in-ui`. Lane T needs only the JSON contract of `AccountView` (Task 5), the command names (Task 11) and the browser outcome (Task 12), all spelled out in Task 15, so it runs **in parallel** with Lane R after Task 1's approval. Its tests mock the commands. At most these two dev lanes run at once.
- **Merge order:** Lane R's PR merges first. Lane T then rebases onto `origin/main` (Task 21, lead), adds the cross-language pins, re-runs its gates and merges. Tasks 22–24 are lead-only and run on builds of the merged `main`.
- **Worktree creation** (lead, from the workspace root):

```bash
WS=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io
git -C $WS/repos/desktop fetch origin
git -C $WS/repos/desktop worktree add ~/code/bb-worktrees/desktop-1747-r -b feat/1747-first-run-sign-in origin/main
git -C $WS/repos/desktop worktree add ~/code/bb-worktrees/desktop-1747-t -b feat/1747-first-run-sign-in-ui origin/main
```

The variables, both loop commands and the lane rules are in Global Constraints. Task 0 runs `bun install --frozen-lockfile` in both lanes' trees.

---

## Task 0: Precondition — spec A is on `main`, and every interface this plan consumes is there (lead)

**Lead only.** No code. Stops the plan if spec A is not merged or a consumed interface is missing.

**Files:**
- Create: `$EVID/t0-interfaces.sh`, `$EVID/t0-interfaces.tsv`, `$EVID/t0-signatures.txt`, `$EVID/t0-record.md`, `$EVID/t0-baseline-*.log` (workspace evidence, lead commits)

**Interfaces:**
- Consumes: spec A as merged. Every name below is how it exists on `feat/1834-finder-reconciler` / `feat/1834-finder-reconciler-ui` (read 2026-10-07) or how spec A Task 12's rulings require it.
- Produces: the drift list (any name that moved or was renamed, with its new spelling); `t0-record.md` (the facts Tasks 9–13 branch on: `SESSION_TRANSITION` scope, the generation contract and its bump sites, the six `spawn_bound_engine` callers, the startup 401's behaviour, T11-M5, the signature table); and the baselines `B_lib`, `B_bun` and the clippy and lockfile counts every later gate compares against.

- [ ] **Step 1: Prove spec A's two lanes are merged**

```bash
D=$WS/repos/desktop; mkdir -p $EVID
git -C $D fetch origin
for head in feat/1834-finder-reconciler feat/1834-finder-reconciler-ui; do
  gh pr list -R beebeeb-io/desktop --state merged --head "$head" --json number,headRefName,mergedAt,mergeCommit --jq '.[] | "\(.number) \(.headRefName) \(.mergedAt) \(.mergeCommit.oid)"'
done > $EVID/t0-spec-a-prs.txt; cat $EVID/t0-spec-a-prs.txt
while read -r num head merged oid; do
  git -C $D merge-base --is-ancestor "$oid" origin/main && echo "$head on main ($oid)" || echo "$head NOT on main"
done < $EVID/t0-spec-a-prs.txt
```

Expected: one line each for `feat/1834-finder-reconciler` and `feat/1834-finder-reconciler-ui`, both "on main". If either is missing or "NOT on main": **stop**, write "Blocked because: spec A lane <name> is not merged" in the task file, and do not dispatch anything else.

- [ ] **Step 2: Write the interface check**

Create `$EVID/t0-interfaces.sh`:

```bash
#!/usr/bin/env bash
# Spec C Task 0: every spec A interface this plan consumes, grepped at origin/main.
# Output: one TSV row per interface: status, file, name. Exit 1 when any is missing.
set -uo pipefail
D=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop
REV=origin/main
GREP=/usr/bin/grep
missing=0
check() { # file, fixed-string pattern, label
  if git -C "$D" grep -q -F -- "$2" "$REV" -- "$1"; then printf 'ok\t%s\t%s\n' "$1" "$3"
  else printf 'MISSING\t%s\t%s\n' "$1" "$3"; missing=$((missing + 1)); fi
}
R=src-tauri/src
check $R/runner.rs 'pub struct AuthHealth' AuthHealth
check $R/runner.rs 'pub fn is_expired(&self) -> bool' AuthHealth::is_expired
check $R/runner.rs 'fn note_result(&self, error: Option<&anyhow::Error>)' AuthHealth::note_result
check $R/state_db.rs 'pub fn owner(&self) -> Result<Option<crate::account_binding::Identity>>' StateDb::owner
check $R/state_db.rs 'pub fn purge_all_local_state(&self)' StateDb::purge_all_local_state
check $R/state_db.rs 'pub fn clear_account_data(&self, owe_finder_removal: bool)' StateDb::clear_account_data
check $R/state_db.rs 'const ACCOUNT_TABLES:' ACCOUNT_TABLES
check $R/state_db.rs 'const DEVICE_TABLES:' DEVICE_TABLES
check $R/state_db.rs 'fn every_table_is_classified_for_the_account_binding' classification-test
check $R/state_db.rs 'pub fn finish_windows_signout(&self)' StateDb::finish_windows_signout
check $R/account_binding.rs 'pub struct Identity' account_binding::Identity
check $R/account_binding.rs 'pub fn same_account(a: &Identity, b: &Identity) -> Option<bool>' account_binding::same_account
check $R/reauth.rs 'pub struct LocalTraces' reauth::LocalTraces
check $R/reauth.rs 'pub fn any(&self) -> bool' LocalTraces::any
check $R/lib.rs 'fn authorize_engine_start(' authorize_engine_start
check $R/lib.rs 'enum StartPermit' StartPermit
check $R/lib.rs 'enum EngineStart {' EngineStart
check $R/lib.rs 'fn start_engine_bound(' start_engine_bound
check $R/lib.rs 'fn spawn_bound_engine(' spawn_bound_engine
check $R/lib.rs 'async fn start_check_engine(' start_check_engine
check $R/lib.rs 'async fn ensure_sync_root_and_engine(' ensure_sync_root_and_engine
check $R/lib.rs 'async fn persist_sync_root_and_start_engine(' persist_sync_root_and_start_engine
check $R/lib.rs 'async fn pick_sync_root(' pick_sync_root
check $R/lib.rs 'async fn start_engine_if_possible(' start_engine_if_possible
check $R/lib.rs 'async fn start_engine_for_pending_finder_install(' start_engine_for_pending_finder_install
check $R/lib.rs 'async fn stop_engine_in_slot(' stop_engine_in_slot
check $R/lib.rs 'engine_stop_unconfirmed' engine_stop_unconfirmed
check $R/lib.rs 'async fn probe_startup_session(' probe_startup_session
check $R/lib.rs 'fn discard_unusable_startup_session(' discard_unusable_startup_session
check $R/lib.rs 'async fn restore_session_on_startup(' restore_session_on_startup
check $R/lib.rs 'async fn open_reauth_window(' open_reauth_window
check $R/lib.rs 'fn open_onboarding_window_impl(' open_onboarding_window_impl
check $R/lib.rs 'struct LoginOutcome' LoginOutcome
check $R/lib.rs 'fn login_outcome_json_is_the_frontends_contract' login_outcome_json_is_the_frontends_contract
check $R/lib.rs 'enum SignInSettlement' SignInSettlement
check $R/lib.rs 'const SIGN_IN_ACCOUNT_UNKNOWN' SIGN_IN_ACCOUNT_UNKNOWN
check $R/lib.rs 'fn pending_changes_count(' pending_changes_count
check $R/lib.rs 'fn keychain_vault_key_present(' keychain_vault_key_present
check $R/lib.rs 'fn keys_arrived(' keys_arrived
check $R/lib.rs 'fn notify_finder(' notify_finder
check $R/lib.rs 'async fn clear_session_impl(' clear_session_impl
check $R/lib.rs 'async fn lock_vault(' lock_vault
check $R/lib.rs 'async fn unlock_vault(' unlock_vault
check $R/lib.rs 'async fn desktop_unlock_with_recovery_phrase(' desktop_unlock_with_recovery_phrase
check $R/lib.rs 'async fn apply_session(' apply_session
check $R/lib.rs 'async fn desktop_login(' desktop_login
check $R/lib.rs 'async fn desktop_login_2fa(' desktop_login_2fa
check $R/lib.rs 'fn gather_local_facts(' gather_local_facts
check $R/lib.rs 'struct LocalSources' LocalSources
check $R/lib.rs 'fn identity_of_session(' identity_of_session
check $R/lib.rs 'fn load_session_token_from_keychain(' load_session_token_from_keychain
check $R/lib.rs 'fn show_macos_settings_window(' show_macos_settings_window
check $R/lib.rs 'fn show_compact_app_window_with_nav(' show_compact_app_window_with_nav
check $R/lib.rs 'fn setup_tray(' setup_tray
check $R/lib.rs 'fn attach_tray_status_listener' attach_tray_status_listener
check $R/lib.rs 'fn build_tray_menu(' build_tray_menu
check $R/lib.rs 'async fn check_for_updates_now(' check_for_updates_now
check $R/lib.rs 'async fn install_update(' install_update
check $R/lib.rs 'pub finder_setup: std::sync::OnceLock<finder_setup::driver::FinderSetupHandle>' AppState.finder_setup
check $R/lib.rs 'impl Default for AppState' AppState-Default
check $R/lib.rs 'static SESSION_TRANSITION' SESSION_TRANSITION
check $R/lib.rs 'session_generation' session-generation-API-from-Task-12
check $R/finder_setup/core.rs 'pub enum Trigger' core::Trigger
check $R/finder_setup/core.rs 'KeysArrived,' Trigger::KeysArrived
check $R/finder_setup/core.rs 'Lock,' Trigger::Lock
check $R/finder_setup/core.rs 'held: bool' CoreState.held
check $R/finder_setup/core.rs 'AddDomain,' Op::AddDomain
check $R/finder_setup/core.rs 'fn on_trigger(' core::on_trigger
check $R/finder_setup/core.rs 'fn on_check_result(' core::on_check_result
check $R/finder_setup/core.rs 'pub fn step(' core::step
check $R/finder_setup/core.rs 'pub fn next_op(' core::next_op
check $R/finder_setup/error.rs 'pub const ENGINE_STOP_UNCONFIRMED: i64 = 7;' app_code-7-is-the-last
check $R/finder_setup/error.rs 'pub fn app(code: i64' FpError::app
check $R/finder_setup/driver.rs 'Lock { ack: oneshot::Sender<()> }' driver::Event::Lock
check $R/finder_setup/driver.rs 'pub async fn lock(&self, timeout: Duration)' FinderSetupHandle::lock
check $R/finder_setup/driver.rs 'pub trait Clock' driver::Clock
check $R/finder_setup/driver.rs 'pub struct SystemClock' driver::SystemClock
check $R/finder_setup/macos_ports.rs 'Op::AddDomain =>' MacosPorts-AddDomain-arm
check $R/finder_setup/launch_location.rs 'Applications,' LaunchLocation::Applications
check $R/lifecycle_log.rs 'pub enum LifecycleEvent' LifecycleEvent
check $R/lifecycle_log.rs 'pub fn format_line(' lifecycle_log::format_line
check $R/browser_login.rs 'pub async fn start_browser_login(' start_browser_login
check $R/browser_login.rs 'async fn run_handoff(' run_handoff
check $R/link_health.rs 'pub fn classify_reqwest(' link_health::classify_reqwest
check $R/api_client.rs 'fn provenance_headers()' api_client::provenance_headers
check $R/engine_status.rs 'pub fn tray_tooltip(' engine_status::tray_tooltip
check $R/surfaces/policy.rs 'pub fn startup_surface(' surfaces::policy::startup_surface
check $R/test_sandbox.rs 'pub(crate) fn dir(' test_sandbox::dir
check src/onboardingSignIn.ts 'export function settledFrom(' settledFrom
check src/onboardingSignIn.ts 'export type SignInSettled' SignInSettled
check src/Onboarding.tsx 'const afterSignIn' afterSignIn
check src/Onboarding.tsx 'function AccountSwitchStep(' AccountSwitchStep
check src/Onboarding.tsx 'function UnlockStep(' UnlockStep
check src/Onboarding.tsx 'function MacFinderStep(' MacFinderStep
check src/Onboarding.tsx 'function FinderInstallStep(' FinderInstallStep
check src/Onboarding.tsx 'function SignInStep(' Onboarding-SignInStep
check src/AuthExpiredBanner.tsx 'Sync is paused until you sign in again.' AuthExpiredBanner-wording
check src/finderSetup.ts 'export function useFinderSetup(' useFinderSetup
check src/finderSetupCopy.ts 'export const FINDER_REASON_COPY' FINDER_REASON_COPY
check src/finderSetupCopy.ts 'export function finderSetupPresentation(' finderSetupPresentation
check src/browserLoginCopy.ts 'export const BROWSER_SIGN_IN_INTRO' BROWSER_SIGN_IN_INTRO
check src/accountSwitchCopy.ts 'export function accountSwitchBody(' accountSwitchBody
check src/accountSwitchCopy.ts 'export const SIGN_IN_OUTCOME_UNREADABLE' SIGN_IN_OUTCOME_UNREADABLE
check src/desktopApi.ts 'export const BILLING_URL' desktopApi.BILLING_URL
check src/desktopApi.ts 'export async function openUrl(' desktopApi.openUrl
check src/macSettingsModel.ts 'export const HELP_URL' macSettingsModel.HELP_URL
check src/macSettingsModel.ts 'export function settingsTabFromSearch(' settingsTabFromSearch
check src/windows/manualUpdateCheck.ts 'export const desktopUpdateCheck' desktopUpdateCheck
check src/WindowsFirstRun.tsx 'function SignInStep(' WindowsFirstRun-SignInStep
check src/pages/SyncFolder.tsx 'export default function' SyncFolder-page
check tests/fixtures/componentHarness.ts 'export function mount(' componentHarness.mount
# Names the plan also consumes (plan review I3).
check $R/surfaces/policy.rs 'pub enum Platform' surfaces::policy::Platform
check $R/surfaces/policy.rs 'Macos,' Platform::Macos
check $R/surfaces/policy.rs 'Windows,' Platform::Windows
check $R/surfaces/policy.rs 'Linux,' Platform::Linux
check $R/runner.rs 'pub fn api_base_url(' runner::api_base_url
check $R/lib.rs 'fn state_db_from_state_dir(' state_db_from_state_dir
check $R/state_paths.rs 'fn beebeeb_state_dir(' state_paths::beebeeb_state_dir
check $R/state_paths.rs 'STATE_DB_FILENAME' state_paths::STATE_DB_FILENAME
check $R/lifecycle_log.rs 'pub fn event(' lifecycle_log::event
check $R/lifecycle_log.rs 'fn token(' lifecycle_log::token
check $R/lifecycle_log.rs 'SignedOut,' LifecycleEvent::SignedOut
check $R/diagnostic_redaction.rs 'pub struct KnownNames' KnownNames
check $R/lib.rs 'async fn settle_sign_in(' settle_sign_in
check $R/lib.rs 'fn revoke_desktop_session(' revoke_desktop_session
check $R/lib.rs 'struct AuthorizeFixture' test-AuthorizeFixture
check $R/lib.rs 'fn with_session(' test-AuthorizeFixture::with_session
check $R/lib.rs 'fn local_data_of(' test-local_data_of
check $R/lib.rs 'fn for_test(' test-LocalDataPaths/LocalSources::for_test
check $R/runner.rs 'fn for_test_with_task(' test-EngineRunner::for_test_with_task
check $R/lib.rs 'fn body_between' test-body_between
check $R/lib.rs 'fn production_source()' test-production_source
check src/desktopApi.ts 'export async function command<' desktopApi.command
check src/desktopApi.ts 'export type CommandResult' desktopApi.CommandResult
check src/desktopApi.ts 'export function commandUnavailableLabel(' desktopApi.commandUnavailableLabel
check src/desktopApi.ts 'export interface DesktopLoginResult' desktopApi.DesktopLoginResult
check src/finderSetupCopy.ts 'export const FINDER_SETUP_TITLE' FINDER_SETUP_TITLE
check src/ManualUpdateFeedback.tsx 'export default function ManualUpdateFeedback(' ManualUpdateFeedback-default-export
check src/Logo.tsx 'export function Wordmark(' Logo.Wordmark
check src/windows/ui.tsx 'export function useToast(' windows/ui.useToast
echo "missing=$missing"
[ "$missing" -eq 0 ]
```

- [ ] **Step 2b: Record the signatures the plan's code calls**

A changed signature surfaces as a compile error in the middle of a task; read them all now. For each function below, print its signature as it is on `main`:

```bash
for sig in 'async fn start_engine_if_possible(' 'fn identity_of_session(' 'async fn settle_sign_in(' 'fn revoke_desktop_session(' \
           'pub fn format_line(' 'fn account_mismatch(' 'fn authorize_engine_start(' 'fn start_engine_bound(' \
           'async fn stop_engine_in_slot(' 'fn note_result(' 'fn state_db_from_state_dir(' 'fn load_session_token_from_keychain(' \
           'pub async fn start_browser_login(' 'pub fn startup_surface('; do
  echo "=== $sig"; git -C $D grep -n -A6 -F -- "$sig" origin/main -- src-tauri/src | /usr/bin/grep -v '^--$' | head -8
done > $EVID/t0-signatures.txt 2>&1; echo "rc=$?"
```

Compare each with the shape the plan assumes, and write the result into the "Signatures" table of `t0-record.md` ("as assumed", or the actual line):

| Function | The plan assumes |
|---|---|
| `start_engine_if_possible` | `(app: tauri::AppHandle, state: &State<'_, AppState>, #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>) -> Result<(), String>` |
| `identity_of_session` | `(email: Option<&str>, profile: Option<&account_dto::AccountProfile>) -> account_binding::Identity` |
| `settle_sign_in` | `(app, state, token: &str, profile: &AccountProfile) -> Result<SignInSettlement, F>` where `F` has `token_stored: bool` and `message: String` |
| `revoke_desktop_session` | `(client: &reqwest::Client, base_url: &str, token: &str)` (async; result ignored) |
| `lifecycle_log::format_line` | `(event: &LifecycleEvent, names: &KnownNames, at: SystemTime) -> String` |
| `LoginOutcome::account_mismatch` | `(pending_changes: u64) -> Self` (`Option<u64>` once T11-M5 is fixed) |
| `authorize_engine_start` | `(state: &AppState, acct: &account::AccountRuntime, engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>, paths: &LocalDataPaths) -> Result<StartPermit, String>` |
| `start_engine_bound` | `(state, acct, engine_slot: &mut MutexGuard<'_, Option<EngineRunner>>, paths, spawn: impl FnOnce(PathBuf, Zeroizing<String>, Zeroizing<[u8; 32]>) -> EngineRunner) -> Result<EngineStart, String>` |
| `stop_engine_in_slot` | `(acct: &account::AccountRuntime, engine: EngineRunner) -> runner::AbortOutcome` (with `is_stopped()`) |
| `AuthHealth::note_result` | `(&self, error: Option<&anyhow::Error>)` |
| `state_db_from_state_dir` | `(state_dir: &std::path::Path) -> Result<Option<state_db::StateDb>, String>` |
| `load_session_token_from_keychain` | `(account_id: &str) -> Result<Option<String>, String>` |
| `start_browser_login` | `(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String>` (Task 12 changes the `Ok` type) |
| `surfaces::policy::startup_surface` | `(platform: Platform, no_sync_root: bool) -> StartupSurface` (Task 13 adds two parameters) |

- [ ] **Step 3: Run it and record the drift**

```bash
bash $EVID/t0-interfaces.sh > $EVID/t0-interfaces.tsv 2>&1; echo "rc=$?"; /usr/bin/grep -c '^ok' $EVID/t0-interfaces.tsv; /usr/bin/grep '^MISSING' $EVID/t0-interfaces.tsv
```

Expected: `rc=0`, the `ok` count equals the number of `check` lines (count them: `/usr/bin/grep -c '^check ' $EVID/t0-interfaces.sh`), no `MISSING` line.

For each `MISSING` row, find where the name went (`git -C $D log origin/main -S '<old name>' --oneline -- <file>`, then `git grep` for the symbol's new spelling). Write a "Drift" table in the task Notes: old name, new name, file. **A renamed interface is substituted throughout this plan by the executor; a removed one stops the plan** (write "Blocked because: <name> is not on main" and queue a decision).

The generation row is special (Spec issue 15). Find its real name:

```bash
git -C $D grep -n -E 'fn session_generation|SESSION_GENERATION|fn .*generation\(' origin/main -- src-tauri/src/lib.rs > $EVID/t0-generation.txt; cat $EVID/t0-generation.txt
```

Record the generation contract in `t0-record.md` (the plan calls it `crate::session_generation() -> u64`):
- **Name and type** of the read function, and whether it returns a value that only ever grows.
- **Bump sites**: every place that moves it (`git -C $D grep -n '<the bump function or counter>' origin/main -- src-tauri/src/lib.rs`), named by the enclosing function. C-D5 needs a bump in every session transition: each sign-in install, sign-out (`clear_session_impl`), lock, unlock and the startup restore's install.
- **Order inside `clear_session_impl`**: the bump must come before `lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedOut)` (Task 11 clears the account view right after that line). Print the function's body (`git -C $D show origin/main:src-tauri/src/lib.rs | awk '/async fn clear_session_impl\(/,/^}/' > $EVID/t0-clear-session-body.txt`) and record the two line numbers.
- **Order inside each sign-in install** (`apply_session`, and each unlock command that installs a key): the bump comes before that function's `start_engine_if_possible` call. Task 9's gate is closed for a generation the account view has not derived for, and that order is what makes a sign-in's own engine start find it closed. Record the line numbers.
- **Scope**: whether the read function is process-wide (a static) or per `AppState`. Task 9's tests are written for a process-wide value that other tests may move.

If no such function exists, or it does not grow monotonically, or a transition above does not bump it, or a bump comes after the `SignedOut` line or after a sign-in's `start_engine_if_possible`: stop and queue the decision "spec C needs the session generation as C-D5 describes; add it in spec C (C-D5 fallback) or wait for spec A".

- [ ] **Step 4: Record the facts later tasks branch on**

```bash
git -C $D grep -n 'pending_changes: Option<u64>' origin/main -- src-tauri/src/lib.rs > $EVID/t0-m5.txt; echo "m5 rc=$?"
git -C $D show origin/main:src-tauri/src/lib.rs | awk '/fn discard_unusable_startup_session\(/,/^}/' > $EVID/t0-discard-body.txt; wc -l < $EVID/t0-discard-body.txt
git -C $D show origin/main:src-tauri/src/lib.rs | /usr/bin/grep -n -B2 'static SESSION_TRANSITION' > $EVID/t0-session-transition.txt; cat $EVID/t0-session-transition.txt
git -C $D show origin/main:src-tauri/src/lib.rs | awk '/async fn start_engine_if_possible\(/,/\) -> Result/' > $EVID/t0-start-engine-signature.txt; cat $EVID/t0-start-engine-signature.txt
git -C $D grep -n 'spawn_bound_engine(' origin/main -- src-tauri/src/lib.rs > $EVID/t0-spawn-callers.txt; cat $EVID/t0-spawn-callers.txt
```

Write each answer into `t0-record.md`:
- **T11-M5:** `t0-m5.txt` non-empty means fixed on `main` (Tasks 12 and 16 skip their M5 steps); empty means open (they run them).
- **The startup 401 (C-R2 (b), spec A Task 12):** read `t0-discard-body.txt`. It keeps the owner record (no call that purges `state.db` or deletes the owner) and the vault key (it clears the session token only, not the whole Keychain trio: no `clear_keychain_session(` call). Record "keeps owner and key: yes", naming the calls you read. If it clears the key or the owner, stop: spec C's Session ended after a relaunch depends on it.
- **`SESSION_TRANSITION`:** "Windows only" when the line above `static SESSION_TRANSITION` is `#[cfg(target_os = "windows")]`, else "every platform". Also record whether `start_engine_if_possible`'s `_transition` parameter is still `#[cfg(target_os = "windows")]` (`t0-start-engine-signature.txt`). Tasks 9 and 11 branch on both.
- **`spawn_bound_engine` callers:** from `t0-spawn-callers.txt`, name the enclosing function of each production call (not the definition, not a test). Expected exactly six: `start_engine_if_possible`, `ensure_sync_root_and_engine`, `persist_sync_root_and_start_engine`, `pick_sync_root`, `start_engine_for_pending_finder_install`, `start_check_engine`. A seventh caller stops Task 9 until the lead rules how it treats `AccountBlocked`.

- [ ] **Step 5: Allocate the worktrees and record the baselines**

Run the worktree commands in "Lanes, branches and order". Then from Lane R's fresh tree:

```bash
cd $WTR && git log --oneline -1 > $EVID/t0-baseline-head.txt
cd $WTR/src-tauri && $LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t0-baseline-clippy.log 2>&1; echo "rc=$?"
/usr/bin/grep -c '^warning' $EVID/t0-baseline-clippy.log > $EVID/t0-baseline-clippy-warnings.txt; cat $EVID/t0-baseline-clippy-warnings.txt
SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/t0-baseline-cargo-test.log 2>&1; echo "rc=$?"
rm -rf "$SCRATCH"
/usr/bin/grep "test result:" $EVID/t0-baseline-cargo-test.log
/usr/bin/grep -c '^\[\[package\]\]' Cargo.lock > $EVID/t0-baseline-lock-packages.txt; cat $EVID/t0-baseline-lock-packages.txt
cd $WTR && bun install --frozen-lockfile > $EVID/t0-bun-install-r.log 2>&1; echo "rc=$?"
cd $WTT && bun install --frozen-lockfile > $EVID/t0-bun-install-t.log 2>&1; echo "rc=$?"
bun test > $EVID/t0-baseline-bun-test.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t0-baseline-bun-test.log
shasum -a 256 bun.lock > $EVID/t0-baseline-bun-lock.sha256
```

Both lanes' trees now have `node_modules` (Lane R's `tauri build` in Tasks 23–24 needs it too; the lanes themselves install nothing). Write into `t0-record.md` under "Baseline (origin/main <sha>)": the head, `B_lib` (the `--lib` binary's `test result:` count), every other binary's `test result:` line, the clippy warning count, the `[[package]]` count, `B_bun` (the pass count) and the `bun.lock` hash.

---

## Task 1: Design before code — the mocks and the icon assets (spec §12)

**Design lane.** The implementer draws; **the lead approves before Tasks 2–21 are dispatched.** No product code.

**Files:**
- Create (workspace, lead commits): `$WS/design/desktop-first-run/macos-account-window.html`, `$WS/design/desktop-first-run/settings-notices.html`, `$WS/design/desktop-first-run/windows.html`, `$WS/design/desktop-first-run/menu-bar-icon.html`, `$WS/design/desktop-first-run/tray-icon/source/make-ico.py`, `$WS/design/desktop-first-run/README.md`
- Read only (workspace, already committed in `39494ebcf`): `$WS/design/desktop-first-run/tray-icon/tray-template-disconnected.png` (22×22), `tray-template-disconnected@2x.png` (44×44), `tray-template-disconnected.svg`, `source/` (the generator scripts and fitted glyph parameters)
- Create (desktop, Lane R branch): `src-tauri/icons/tray-template-disconnected@2x.png`, `src-tauri/icons/tray-disconnected.ico`
- Evidence: `$EVID/t1-*.png`, `$EVID/t1-assets.log`

**Interfaces:**
- Consumes: today's `src-tauri/icons/tray-template@2x.png` (sha256 `739f5556…95d4`), `src-tauri/icons/icon.ico` (6 PNG frames: 16, 32, 48, 64, 128, 256), `src-tauri/icons/icon.png`; the variant B files.
- Produces: the approved screens every UI task builds against, and the two icon files Task 14 embeds by these exact names.

- [ ] **Step 1: Copy the variant B template into the desktop repo, byte-checked**

The approved files (C4b, "ye B") are committed in the workspace repo at `$WS/design/desktop-first-run/tray-icon/` (commit `39494ebcf`). Their SHA-256 sums, recorded 2026-10-07 and checked against those copies:

| File | SHA-256 |
|---|---|
| `tray-template-disconnected@2x.png` (44×44) | `e0817422ca34cd757a9996fc1f0cece964764e2f88c30f4a573f4fc3741ab313` |
| `tray-template-disconnected.png` (22×22) | `4302b9d159d3baef5defab2be4a0a017211d0a33f3b32e1f0cda7529c5b67cbc` |
| `tray-template-disconnected.svg` | `b815b59638f2d5531af4b58c2bac734f9bcbb28c97537b718534565c937c6877` |

```bash
WTR=~/code/bb-worktrees/desktop-1747-r; DES=$WS/design/desktop-first-run; SRC=$DES/tray-icon
git -C $WS log --oneline -1 -- design/desktop-first-run/tray-icon > $EVID/t1-source-commit.txt; cat $EVID/t1-source-commit.txt
shasum -a 256 $SRC/tray-template-disconnected@2x.png $SRC/tray-template-disconnected.png $SRC/tray-template-disconnected.svg > $EVID/t1-source-sums.txt 2>&1; echo "rc=$?"; cat $EVID/t1-source-sums.txt
```

If `rc≠0` or any sum differs from the table: **stop** (Spec issue 18). Report BLOCKED; never redraw the icon. Otherwise copy only the 44×44 template into the desktop repo (the 22×22 file, the SVG and `source/` stay in the workspace as reference, C-I3):

```bash
cp $SRC/tray-template-disconnected@2x.png $WTR/src-tauri/icons/tray-template-disconnected@2x.png
shasum -a 256 $WTR/src-tauri/icons/tray-template-disconnected@2x.png > $EVID/t1-assets.log; cat $EVID/t1-assets.log
```

Expected: the copy's sum equals the table's `@2x` row.

- [ ] **Step 2: Check the template against today's b**

Black and alpha only, 44×44, and identical to today's `tray-template@2x.png` outside the slash's knockout band (C-I3). The band: distance from the line `x + y = 44` (pixel centres) at most 4.6 px plus 1 px of anti-aliasing.

```bash
python3 - "$WTR/src-tauri/icons/tray-template@2x.png" "$WTR/src-tauri/icons/tray-template-disconnected@2x.png" > $EVID/t1-template-check.log 2>&1 <<'EOF'
import sys, math
from PIL import Image
old, new = (Image.open(p).convert("RGBA") for p in sys.argv[1:3])
assert old.size == new.size == (44, 44), (old.size, new.size)
colour = sum(1 for (r, g, b, a) in new.getdata() if a and (r, g, b) != (0, 0, 0))
outside = differ = 0
for y in range(44):
    for x in range(44):
        if abs((x + 0.5) + (y + 0.5) - 44) / math.sqrt(2) > 5.6:
            outside += 1
            differ += old.getpixel((x, y)) != new.getpixel((x, y))
print(f"non-black opaque pixels: {colour}; outside the band: {outside} pixels, {differ} differ")
assert colour == 0 and differ == 0
EOF
echo "rc=$?"; cat $EVID/t1-template-check.log
```

Expected: `rc=0`, `non-black opaque pixels: 0; outside the band: N pixels, 0 differ` with N > 1000.

- [ ] **Step 3: Draw the colour crossed `.ico` (C-I4)**

Create `$DES/tray-icon/source/make-ico.py` (beside the template's generator scripts; it uses the same geometry, read from `tray-template-disconnected.svg`: the line `(5.8, 38.2)`→`(38.2, 5.8)`, stroke 5.2, knockout 9.2, on a 44-unit grid where the b is 33.4 tall):

```python
#!/usr/bin/env python3
"""The colour crossed b for Windows and Linux (spec 2026-10-07 C-I4), drawn from today's icon.ico.

Variant B (ruling C4b), scaled from the template's 44-unit grid, where the b's ink is 33.4 units
tall: a slash from bottom-left to top-right at 45 degrees through the centre of the b's ink box,
ends 16.2 units from the centre, stroke 5.2 units (2.6 pt on the 22 pt template), round caps, and
a knockout 2 units wider on each side (1 pt). On the colour icon the knockout is painted in the
icon's own background colour instead of being cut to transparency, so the square stays whole.
Only an overlay is drawn and composited: every pixel outside the slash is today's pixel.

Usage: make-ico.py <today's icon.ico> <out tray-disconnected.ico>
"""
import sys
from PIL import Image, ImageDraw

B_HEIGHT = 33.4
HALF_LENGTH = 16.2 / B_HEIGHT
STROKE = 5.2 / B_HEIGHT
KNOCKOUT = 9.2 / B_HEIGHT
SUPERSAMPLE = 8


def ink_box(frame):
    """The dark b's bounding box on the amber square."""
    pixels = frame.load()
    width, height = frame.size
    points = [(x, y) for y in range(height) for x in range(width)
              if pixels[x, y][3] > 128 and sum(pixels[x, y][:3]) < 200]
    xs, ys = [p[0] for p in points], [p[1] for p in points]
    return min(xs), min(ys), max(xs) + 1, max(ys) + 1


def crossed(frame):
    frame = frame.convert("RGBA")
    size = frame.size[0]
    x0, y0, x1, y1 = ink_box(frame)
    cx, cy, height = (x0 + x1) / 2, (y0 + y1) / 2, y1 - y0
    background = frame.getpixel((0, size // 2))
    ink = frame.getpixel((int(x0 + (x1 - x0) * 0.1), int(cy)))
    s = SUPERSAMPLE
    overlay = Image.new("RGBA", (size * s, size * s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay)
    start = ((cx - HALF_LENGTH * height) * s, (cy + HALF_LENGTH * height) * s)
    end = ((cx + HALF_LENGTH * height) * s, (cy - HALF_LENGTH * height) * s)
    for width, colour in ((KNOCKOUT, background), (STROKE, ink)):
        w = max(1, round(width * height * s))
        draw.line([start, end], fill=colour, width=w)
        for x, y in (start, end):
            draw.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=colour)
    return Image.alpha_composite(frame, overlay.resize((size, size), Image.LANCZOS))


def main(source, target):
    ico = Image.open(source)
    sizes = sorted(ico.ico.sizes())
    frames = [crossed(ico.ico.getimage(size)) for size in sizes]
    largest = frames[-1]
    largest.save(target, format="ICO", sizes=sizes, append_images=frames[:-1])
    print("frames:", sizes)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
```

```bash
python3 $DES/tray-icon/source/make-ico.py $WTR/src-tauri/icons/icon.ico $WTR/src-tauri/icons/tray-disconnected.ico > $EVID/t1-ico.log 2>&1; echo "rc=$?"; cat $EVID/t1-ico.log
python3 - $WTR/src-tauri/icons/icon.ico $WTR/src-tauri/icons/tray-disconnected.ico >> $EVID/t1-ico.log 2>&1 <<'EOF'
import sys
from PIL import Image
old, new = (Image.open(p) for p in sys.argv[1:3])
assert sorted(old.ico.sizes()) == sorted(new.ico.sizes()), (old.ico.sizes(), new.ico.sizes())
for size in sorted(old.ico.sizes()):
    a, b = old.ico.getimage(size).convert("RGBA"), new.ico.getimage(size).convert("RGBA")
    changed = sum(1 for p, q in zip(a.getdata(), b.getdata()) if p != q)
    print(size, "changed pixels:", changed)
    assert 0 < changed < size[0] * size[1] // 2
EOF
echo "rc=$?"; tail -8 $EVID/t1-ico.log
```

Expected: `rc=0` twice; six frames listed, each with some changed pixels and fewer than half the frame changed. Render the 256 and 16 px frames to `$EVID/t1-ico-256.png` and `$EVID/t1-ico-16.png` (`Image.open(ico).ico.getimage((256,256)).save(...)`) for the lead.

- [ ] **Step 4: Draw the mocks**

These mocks are specified in prose, not shown: the list below gives every screen, its exact strings and its states, and the skeleton leaves the token block to be copied. That is the one deliverable in this plan written that way, because it is a drawing gated by the lead. **The lead's approval in Step 7 checks every visible string in the renders word for word against the strings the spec gives for that screen**, not only the layout.

Every mock file starts from this skeleton (tokens copied from the desktop repo's `src/design.css`, the same way the desktop repo's `design/hifi/macos-settings-dialogs.html` does, so the drawing matches the built window; theme from `?theme=light|dark`, else the OS):

```html
<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Beebeeb desktop first run · spec C</title>
<link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap">
<style>
/* Tokens: copied verbatim from repos/desktop/src/design.css (:root and the dark block). */
:root{ /* paste the light tokens */ color-scheme:light; }
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){ /* paste the dark tokens */ color-scheme:dark; }}
:root[data-theme="dark"]{ /* paste the dark tokens */ color-scheme:dark; }
body{margin:0;background:var(--paper-2);color:var(--ink);font:14px/1.5 var(--font-sans);padding:40px clamp(16px,4vw,56px)}
.sheet{display:grid;gap:56px;max-width:1500px;margin:0 auto}
.screen{display:grid;gap:12px}
.screen-head{display:flex;gap:14px;align-items:baseline}
.screen-head .id{font:600 12px var(--font-mono);color:var(--ink-3)}
.screen-head h2{margin:0;font-size:18px;letter-spacing:-.01em}
.pair{display:grid;grid-template-columns:repeat(auto-fit,minmax(560px,1fr));gap:24px}
.frame{border:1px solid var(--line);border-radius:12px;overflow:hidden;box-shadow:var(--shadow-3);background:var(--paper)}
.frame[data-theme]{color:var(--ink)}
.titlebar{height:28px;display:flex;align-items:center;justify-content:center;font:500 12px var(--font-sans);color:var(--ink-2);border-bottom:1px solid var(--line);background:var(--paper-2)}
.window{width:860px;max-width:100%;aspect-ratio:860/640;display:grid;place-items:center}
.mono{font-family:var(--font-mono)}
.note{font:12px/1.5 var(--font-mono);color:var(--ink-3)}
</style>
<script>
const theme = new URLSearchParams(location.search).get('theme');
if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
</script></head><body><main class="sheet">
<!-- One <section class="screen"> per screen; inside .pair, the same frame twice: data-theme="light" and data-theme="dark". -->
</main></body></html>
```

Paste the token blocks from `$WTR/src/design.css` exactly (the `:root` block and its dark counterparts); do not invent values. Each frame carries `data-theme="light"` or `data-theme="dark"` and redeclares the tokens under `.frame[data-theme="dark"]` so both themes render on one page. The account window is 860×640, centred card, no step rail (§11). Amber only on the primary button and on encryption state. Machine text (the device code, addresses) in JetBrains Mono.

Screens, with their exact words (spec §7; typographic ’):

`macos-account-window.html` (title bar text in brackets):
1. **C-R1 sign-in, browser** [Sign in to Beebeeb]: wordmark; heading "Sign in to Beebeeb"; the 1734 intro sentence (`BROWSER_SIGN_IN_INTRO`, unchanged); amber button "Sign in with browser"; text link "Use email and password"; footer "New to Beebeeb? Create an account on beebeeb.io" (the second half is the link) and, smaller, "Quit Beebeeb".
2. **C-R1 waiting for the browser**: today's device-code panel (`browserWaitingInstruction('waiting')`, the code `ABCD-EFGH` in JetBrains Mono, and today's "Browser didn’t open? Visit <the server's `verification_uri`, mono> in any signed-in browser, then type the code there." with the address drawn as a grey mono placeholder, never an invented URL), the button reading "Waiting for browser…".
3. **C-R1 email and password**: today's "Welcome back" / "Sign in to unlock your encrypted vault." form, unchanged, plus "← Back"; same footer.
4. **C-R1 offline**: heading "Sign in to Beebeeb"; in place of the buttons "Can’t reach Beebeeb. Check your connection." and "Try again"; the footer still offers "Create an account on beebeeb.io".
5. **C-R1 with the app on the disk image**: spec A's `not_in_applications` sentence "Beebeeb is running from the disk image. Move it to Applications, then open it again." and "Show in Finder" above the buttons.
6. **C-R2 session ended** [Sign in to Beebeeb]: heading "Sign in again"; "Sync is paused until you sign in again."; the same two methods and footer.
7. **C-R3 update required** [Update Beebeeb]: "Update Beebeeb" — "This version of Beebeeb is too old to connect. Update to continue." — amber "Update now". Second frame: offline variant (the offline line and "Try again" replace "Update now").
8. **C-R5 choose a plan, before the click** [Choose a plan]: "Choose a plan" — "Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself." — amber "Choose a plan on beebeeb.io" — quiet "Sign out".
9. **C-R5 after the click**: the same, plus "Waiting for your plan. This window updates by itself."
10. **C-R5 offline**: the offline line and "Try again" replace the amber button.
11. **Browser won't open** (any link): the address `https://app.beebeeb.io/billing` as selectable mono text, and "Copy link".
12. **Checking (busy)**: the window's busy indicator only, no words.
13. **After sign-in (§7.6)**: "Adding Beebeeb to Finder…"; second frame: spec A's `extension_loading` failure, "macOS hasn’t finished loading Beebeeb’s Finder extension." + "Try again".
14. **The account switch warning** (spec A, unchanged) and, only if Task 0 found T11-M5 open, its unknown-count variant with the sentence in Spec issue 16, marked "NEW COPY — lead approval".

`settings-notices.html`: the macOS Settings Account tab (from the desktop repo's `design/hifi/macos-settings-dialogs.html`), with the notice line under the email and its link in place of "Upgrade"/"Manage plan", once per row: trial ("Free trial until 18 Oct." + "Manage plan"); trial with server copy (the fixture's `trial_end_over_allowance` sentence, verbatim); read-only, both dates ("Read-only since 18 Oct. Files are deleted on 1 Nov unless you choose a plan." + "Choose a plan"); read-only, deletion only ("Read-only. Files are deleted on 1 Dec unless you choose a plan."); read-only, neither ("Read-only. Choose a plan to upload again."); "Read-only." alone (no link; the plan button stays); frozen ("This account is frozen." + "Contact support"); payment failed ("Payment failed. Update your payment details on beebeeb.io." + "Update payment details"); and no notice (the "Manage plan" button as today).

`windows.html`: the `windows-onboarding` window's sign-in step with "Create an account on beebeeb.io" in both modes and the offline line; the same window showing C-R3 and C-R5 before "Files on demand"; `main-app` showing C-R3 and C-R5 in place of its content; the Account view's notice line for each row above, the frozen one replacing today's "Frozen since … Reach support at beebeeb.io …" row.

`menu-bar-icon.html`: today's b and the crossed b at 22 pt and 44 px on a light and a dark macOS menu bar, each with its tooltip ("Beebeeb: Signed out", "Beebeeb: Sign in again", "Beebeeb: Offline"); the colour `.ico` at 16, 32 and 256 px on a light and a dark Windows taskbar.

`README.md`: what each file shows, the spec IDs, that the copy is verbatim from spec §7, and how to view (`open <file>.html?theme=dark`).

- [ ] **Step 5: Render both themes for review**

```bash
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
for f in macos-account-window settings-notices windows menu-bar-icon; do
  for t in light dark; do
    "$CHROME" --headless=new --hide-scrollbars --window-size=1600,6000 --screenshot=$EVID/t1-$f-$t.png "file://$DES/$f.html?theme=$t" > /dev/null 2>&1; echo "$f $t rc=$?"
  done
done
ls -la $EVID/t1-*.png
```

Expected: 8 PNGs, each non-empty. Open every one and check: both themes legible, amber only on primary buttons, every string identical to the table above (copy them from the spec, never retype).

- [ ] **Step 6: Commit the icons on Lane R's branch**

```bash
cd $WTR && git add src-tauri/icons/tray-template-disconnected@2x.png src-tauri/icons/tray-disconnected.ico
git commit -m "desktop: add the disconnected menu-bar icon (variant B) for macOS and Windows/Linux" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/icons/tray-template-disconnected@2x.png src-tauri/icons/tray-disconnected.ico
git show --stat HEAD
```

Expected: exactly the two files. Report the workspace files to the lead (the lead commits them on the workspace repo).

- [ ] **Step 7: Lead approval (lead)**

The lead reviews the 8 renders, the icon checks and the HTML (every string word for word against the spec, see Step 4), then writes in the task Notes: "Mocks approved — lead, <date>" or the changes needed. If a mock and the spec disagree and the lead keeps the mock, the spec is amended in the same change (strike, replace, sign and date). **No code task is dispatched before this line exists.**

---

## Task 2: The vendored contract and the tolerant document parser (`account_view::doc`)

**Lane R.** Pure. No OS, no clock, no network.

**Files:**
- Create (vendored, byte-identical): `contracts/onboarding/` (`README.md`, `schema.v1.json`, `fixtures/*.json`, `invalid/*.json`)
- Create: `src-tauri/src/account_view/mod.rs`, `src-tauri/src/account_view/doc.rs`
- Modify: `src-tauri/src/lib.rs` (module list, above spec A's comment and attribute for `mod finder_setup;`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (`crate::account_view::doc`):
  - `SCHEMA_MAJOR: u64 = 1`
  - `enum Stage { PreAccount, Account }`, `enum ClientStatus { Ok, UpdateRecommended, UpdateRequired, Unknown }`
  - `enum FallbackKind { UpdateApp, UseWeb, ContactSupport, Unknown }` (serde snake_case), `struct Fallback { kind: FallbackKind, url: Option<String> }`
  - `struct Step { id: String, required: bool, status: String, fallback: Option<Fallback> }`
  - `struct Capability { allowed: bool, reason: Option<String> }`, `struct Capabilities { upload: Option<Capability> }`
  - `struct Trial { ends_at: DateTime<Utc> }`, `struct Lifecycle { read_only_since: Option<DateTime<Utc>>, data_deletion_at: Option<DateTime<Utc>> }`
  - `struct Account { state: String, capabilities: Capabilities, trial: Option<Trial>, lifecycle: Option<Lifecycle> }`
  - `struct AccountPart { account: Account, blocking: bool, steps: Vec<Step>, copy: BTreeMap<String, String>, fallback: Option<Fallback>, poll_seconds: Option<u64> }` (Serialize + Deserialize: this is the cached JSON)
  - `struct PreAccount { client: ClientStatus, signup_web_url: Option<String>, fallback: Option<Fallback>, ttl_seconds: Option<u64> }`
  - `struct AccountDoc { client: ClientStatus, part: AccountPart, ttl_seconds: Option<u64> }`
  - `enum Doc { PreAccount(PreAccount), Account(AccountDoc) }` with `stage()`, `client()`, `ttl_seconds()`, `account_part() -> Option<&AccountPart>`, `pre_account() -> Option<&PreAccount>`
  - `enum Unavailable { Unreadable, UnknownSchema }`; `fn parse(bytes: &[u8]) -> Result<Doc, Unavailable>`
  - test helpers (in `doc::tests`, `pub(crate)`): `fixture(name) -> Vec<u8>`, `fixture_names() -> Vec<String>`, `edited(name, impl FnOnce(&mut serde_json::Value)) -> Vec<u8>`, `account_part(name) -> AccountPart`

- [ ] **Step 1: Vendor the contract from the server's `main`**

```bash
WS=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io; WT=~/code/bb-worktrees/desktop-1747-r; EVID=$WS/.claude/tasks/_qa-evidence/1747
git -C $WS/repos/server fetch origin
git -C $WS/repos/server rev-parse origin/main > $EVID/t2-server-sha.txt
git -C $WS/repos/server archive origin/main contracts/onboarding | tar -x -C $WT
CHECK=$(mktemp -d); git -C $WS/repos/server archive origin/main contracts/onboarding | tar -x -C $CHECK
diff -r $CHECK/contracts/onboarding $WT/contracts/onboarding > $EVID/t2-vendor-diff.log 2>&1; echo "rc=$?"; rm -rf "$CHECK"
ls $WT/contracts/onboarding/fixtures | wc -l
```

Expected: `rc=0`, an empty diff log, and at least 20 fixtures. Record the server SHA in the task Notes. If the server has since added `account.needs_plan.desktop.json` (task 1844), it is vendored with the rest; Step 3's test uses it.

- [ ] **Step 2: Create the module root and register it**

`src-tauri/src/account_view/mod.rs`:

```rust
//! Spec 2026-10-07 (desktop first run, sign-in only): one account state, derived in Rust from the
//! session, the server's onboarding document and connectivity. Every window and the menu-bar icon
//! only render it. Pure units (`doc`, `links`, `notice`, `derive`, `policy`) compile and are tested
//! on every platform; `driver` is the one task that fetches and acts.

pub mod doc;
```

In `src-tauri/src/lib.rs`, directly above spec A's two-line comment that starts `// Spec 2026-10-06 (macOS Finder setup reconciler).`, which sits above `#[cfg_attr(not(target_os = "macos"), allow(dead_code))]` and `mod finder_setup;`. Never between that attribute and its module: an item inserted there takes the attribute and leaves `finder_setup` without it.

```rust
// Spec 2026-10-07: the account state the windows and the menu-bar icon render. Wired into the app in
// the plan's Task 11, which removes this allow.
#[allow(dead_code)]
mod account_view;
```

- [ ] **Step 3: Write `doc.rs` with its tests first and `parse` as `todo!()`**

`src-tauri/src/account_view/doc.rs`:

```rust
//! Spec 2026-10-07 §5 C-D3: the onboarding document as this desktop reads it. Tolerant of what it
//! does not read (unknown fields, unknown step ids, unknown states, unknown fallback kinds, and
//! everything a pre_account document says about steps and policy), strict about the shape of what
//! it does read: a known field of the wrong type makes the whole document unreadable (§10,
//! "document unavailable"), never half-applied. Pure: bytes in, `Doc` out. The body itself is never
//! kept, logged or shown (C-D6); only what is parsed here is.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The highest schema major this client understands. Sent as `X-Beebeeb-Onboarding-Schema`.
pub const SCHEMA_MAJOR: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    PreAccount,
    Account,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Ok,
    UpdateRecommended,
    UpdateRequired,
    /// A status this client does not know. Treated like `Ok`: only `UpdateRequired` blocks.
    Unknown,
}

impl ClientStatus {
    fn parse(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "update_recommended" => Self::UpdateRecommended,
            "update_required" => Self::UpdateRequired,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    UpdateApp,
    UseWeb,
    ContactSupport,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fallback {
    pub kind: FallbackKind,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub fallback: Option<Fallback>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub allowed: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Only `upload` is read (§7.5). An absent capability means "not allowed" (the contract's rule).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub upload: Option<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    #[serde(default)]
    pub read_only_since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub data_deletion_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub state: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub trial: Option<Trial>,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
}

/// The account part of an account-stage document: exactly what is cached (C-D5). `client.status` is
/// not in it, so "update required" can only ever come from a fresh document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPart {
    pub account: Account,
    pub blocking: bool,
    pub steps: Vec<Step>,
    pub copy: BTreeMap<String, String>,
    pub fallback: Option<Fallback>,
    pub poll_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreAccount {
    pub client: ClientStatus,
    pub signup_web_url: Option<String>,
    pub fallback: Option<Fallback>,
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDoc {
    pub client: ClientStatus,
    pub part: AccountPart,
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doc {
    PreAccount(PreAccount),
    Account(AccountDoc),
}

impl Doc {
    pub fn stage(&self) -> Stage {
        match self {
            Self::PreAccount(_) => Stage::PreAccount,
            Self::Account(_) => Stage::Account,
        }
    }

    pub fn client(&self) -> ClientStatus {
        match self {
            Self::PreAccount(doc) => doc.client,
            Self::Account(doc) => doc.client,
        }
    }

    pub fn ttl_seconds(&self) -> Option<u64> {
        match self {
            Self::PreAccount(doc) => doc.ttl_seconds,
            Self::Account(doc) => doc.ttl_seconds,
        }
    }

    pub fn account_part(&self) -> Option<&AccountPart> {
        match self {
            Self::Account(doc) => Some(&doc.part),
            Self::PreAccount(_) => None,
        }
    }

    pub fn pre_account(&self) -> Option<&PreAccount> {
        match self {
            Self::PreAccount(doc) => Some(doc),
            Self::Account(_) => None,
        }
    }
}

/// Why a body is not a usable document (§10). An HTTP status is the fetch's business, not this one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    Unreadable,
    UnknownSchema,
}

/// The wire shape, read in two passes: `schema` first (a v2 body may not fit v1's types), then the
/// rest. `steps` and `copy` stay raw until the stage is known: a pre_account document's steps list
/// native signup (C-D3: never read), so nothing in them may make the document unreadable.
#[derive(Deserialize)]
struct Wire {
    stage: String,
    #[serde(default)]
    ttl_seconds: Option<u64>,
    #[serde(default)]
    client: Option<WireClient>,
    #[serde(default)]
    signup: Option<WireSignup>,
    #[serde(default)]
    account: Option<Account>,
    #[serde(default)]
    purchase: Option<WirePurchase>,
    #[serde(default)]
    steps: serde_json::Value,
    #[serde(default)]
    blocking: bool,
    #[serde(default)]
    copy: serde_json::Value,
    #[serde(default)]
    fallback: Option<Fallback>,
}

#[derive(Deserialize)]
struct WireClient {
    status: String,
}

#[derive(Deserialize)]
struct WireSignup {
    #[serde(default)]
    web_url: Option<String>,
}

#[derive(Deserialize)]
struct WirePurchase {
    #[serde(default)]
    checkout: Option<WireCheckout>,
}

#[derive(Deserialize)]
struct WireCheckout {
    #[serde(default)]
    poll_seconds: Option<u64>,
}

pub fn parse(bytes: &[u8]) -> Result<Doc, Unavailable> {
    todo!("Task 2 step 5")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn fixtures_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/onboarding/fixtures")
    }

    pub(crate) fn fixture(name: &str) -> Vec<u8> {
        let path = fixtures_dir().join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    pub(crate) fn fixture_names() -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(fixtures_dir())
            .expect("the vendored fixtures")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".json"))
            .collect();
        names.sort();
        names
    }

    /// A fixture with one edit applied, for the derived cases (unknown states, wrong types, ...).
    pub(crate) fn edited(name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut value: serde_json::Value = serde_json::from_slice(&fixture(name)).unwrap();
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    pub(crate) fn account_part(name: &str) -> AccountPart {
        parse(&fixture(name)).unwrap().account_part().cloned().unwrap_or_else(|| panic!("{name} is not account-stage"))
    }

    #[test]
    fn every_vendored_fixture_parses_into_its_stage() {
        let names = fixture_names();
        assert!(names.len() >= 20, "{names:?}");
        for name in &names {
            let doc = parse(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let expected = if name.starts_with("account.") { Stage::Account } else { Stage::PreAccount };
            assert_eq!(doc.stage(), expected, "{name}");
        }
    }

    #[test]
    fn pre_account_reads_the_status_the_signup_url_and_the_fallback_and_never_the_steps() {
        let Doc::PreAccount(pre) = parse(&fixture("pre_account.desktop.json")).unwrap() else {
            panic!("pre_account.desktop.json is not pre_account");
        };
        assert_eq!(pre.client, ClientStatus::Ok);
        assert_eq!(pre.signup_web_url.as_deref(), Some("https://app.beebeeb.io/signup"));
        assert_eq!(pre.fallback, Some(Fallback { kind: FallbackKind::UseWeb, url: Some("https://app.beebeeb.io/signup".into()) }));
        assert_eq!(pre.ttl_seconds, Some(300));
        let garbage_steps = edited("pre_account.desktop.json", |v| v["steps"] = json!([{ "id": 7, "required": "maybe" }]));
        assert!(parse(&garbage_steps).is_ok(), "a pre_account document's steps are never read");
    }

    #[test]
    fn update_required_comes_from_client_status() {
        assert_eq!(parse(&fixture("client.update_required.ios.json")).unwrap().client(), ClientStatus::UpdateRequired);
        let unknown = edited("account.active.web.json", |v| v["client"]["status"] = json!("future_status"));
        assert_eq!(parse(&unknown).unwrap().client(), ClientStatus::Unknown);
    }

    #[test]
    fn account_documents_carry_the_state_blocking_steps_copy_capabilities_and_dates() {
        let part = account_part("account.needs_plan.ios.json");
        assert_eq!(part.account.state, "needs_plan");
        assert!(part.blocking);
        assert_eq!(part.steps.iter().map(|s| (s.id.as_str(), s.required)).collect::<Vec<_>>(), vec![("verify_email", true)]);

        let part = account_part("account.trial_ended.ios.json");
        assert_eq!(part.account.trial.as_ref().unwrap().ends_at.to_rfc3339(), "2026-10-18T09:00:00+00:00");
        let lifecycle = part.account.lifecycle.as_ref().unwrap();
        assert_eq!(lifecycle.read_only_since.unwrap().to_rfc3339(), "2026-10-18T09:00:00+00:00");
        assert_eq!(lifecycle.data_deletion_at.unwrap().to_rfc3339(), "2026-11-01T09:00:00+00:00");
        assert!(part.copy.contains_key("trial_ended_over_allowance"));
        assert_eq!(part.account.capabilities.upload, Some(Capability { allowed: false, reason: Some("trial_ended".into()) }));

        let part = account_part("account.lapsed.ios.json");
        assert_eq!(part.account.lifecycle.as_ref().unwrap().read_only_since, None);
        assert_eq!(part.account.lifecycle.as_ref().unwrap().data_deletion_at.unwrap().to_rfc3339(), "2026-12-01T09:00:00+00:00");
    }

    /// Spec §14: the needs_plan row runs on the iOS fixture until the server adds a desktop one, then on both.
    #[test]
    fn needs_plan_parses_from_every_needs_plan_fixture_there_is() {
        let names: Vec<String> = fixture_names().into_iter().filter(|n| n.starts_with("account.needs_plan.") && !n.contains("coupon")).collect();
        assert!(names.contains(&"account.needs_plan.ios.json".to_string()), "{names:?}");
        for name in &names {
            let part = account_part(name);
            assert!(part.blocking && part.account.state == "needs_plan", "{name}");
        }
    }

    #[test]
    fn poll_seconds_comes_from_purchase_checkout() {
        assert_eq!(account_part("account.trialing.desktop.json").poll_seconds, Some(3));
        assert_eq!(account_part("account.active.web.json").poll_seconds, None);
    }

    #[test]
    fn unknown_states_steps_kinds_and_fields_are_tolerated() {
        assert!(parse(&fixture("forward_compat.unknown_step.ios.json")).is_ok());
        let future = edited("account.trial_ended.ios.json", |v| {
            v["account"]["state"] = json!("future_state");
            v["steps"] = json!([{ "id": "future_step", "status": "todo", "required": true, "ui": "action",
                                   "fallback": { "kind": "future_kind", "url": "https://app.beebeeb.io/x" } }]);
            v["future_top_level"] = json!({ "anything": 1 });
        });
        let part = parse(&future).unwrap().account_part().cloned().unwrap();
        assert_eq!(part.account.state, "future_state");
        assert_eq!(part.steps[0].fallback.as_ref().unwrap().kind, FallbackKind::Unknown);
    }

    #[test]
    fn a_schema_this_client_does_not_know_is_unknown_schema() {
        let v2 = edited("account.active.web.json", |v| {
            v["schema"] = json!(2);
            v["account"] = json!("re-typed in a later major");
        });
        assert_eq!(parse(&v2), Err(Unavailable::UnknownSchema));
    }

    #[test]
    fn garbage_and_wrong_types_are_unreadable() {
        assert_eq!(parse(b"<html>502 Bad Gateway</html>"), Err(Unavailable::Unreadable));
        assert_eq!(parse(br#"{"stage":"account"}"#), Err(Unavailable::Unreadable), "no schema");
        let blocking = edited("account.active.web.json", |v| v["blocking"] = json!("yes"));
        assert_eq!(parse(&blocking), Err(Unavailable::Unreadable));
        let no_account = edited("account.active.web.json", |v| {
            v.as_object_mut().unwrap().remove("account");
        });
        assert_eq!(parse(&no_account), Err(Unavailable::Unreadable));
        let bad_date = edited("account.trialing.desktop.json", |v| v["account"]["trial"]["ends_at"] = json!("soon"));
        assert_eq!(parse(&bad_date), Err(Unavailable::Unreadable));
        let stage = edited("account.active.web.json", |v| v["stage"] = json!("future_stage"));
        assert_eq!(parse(&stage), Err(Unavailable::Unreadable));
        let copy = edited("account.trial_ended.ios.json", |v| v["copy"]["trial_ended_over_allowance"] = json!(42));
        assert_eq!(parse(&copy), Err(Unavailable::Unreadable));
    }

    #[test]
    fn the_cached_part_round_trips_and_never_holds_the_client_status() {
        for name in fixture_names().iter().filter(|n| n.starts_with("account.")) {
            let part = account_part(name);
            let json = serde_json::to_string(&part).unwrap();
            assert!(!json.contains("\"client\""), "{name}: {json}");
            assert_eq!(serde_json::from_str::<AccountPart>(&json).unwrap(), part, "{name}");
        }
    }
}
```

- [ ] **Step 4: Run the tests to see them fail**

Run: the loop command with filter `account_view::doc`.
Expected: the build succeeds and every test panics with `not yet implemented: Task 2 step 5`; `test result: FAILED. 0 passed; 10 failed`. Paste the summary into the Notes.

- [ ] **Step 5: Implement `parse`**

Replace the `todo!()` body:

```rust
pub fn parse(bytes: &[u8]) -> Result<Doc, Unavailable> {
    #[derive(Deserialize)]
    struct Head {
        schema: u64,
    }
    let head: Head = serde_json::from_slice(bytes).map_err(|_| Unavailable::Unreadable)?;
    if head.schema != SCHEMA_MAJOR {
        return Err(Unavailable::UnknownSchema);
    }
    let wire: Wire = serde_json::from_slice(bytes).map_err(|_| Unavailable::Unreadable)?;
    let client = wire.client.map_or(ClientStatus::Ok, |client| ClientStatus::parse(&client.status));
    match wire.stage.as_str() {
        "pre_account" => Ok(Doc::PreAccount(PreAccount {
            client,
            signup_web_url: wire.signup.and_then(|signup| signup.web_url),
            fallback: wire.fallback,
            ttl_seconds: wire.ttl_seconds,
        })),
        "account" => {
            let account = wire.account.ok_or(Unavailable::Unreadable)?;
            let steps: Vec<Step> = if wire.steps.is_null() {
                Vec::new()
            } else {
                serde_json::from_value(wire.steps).map_err(|_| Unavailable::Unreadable)?
            };
            let copy: BTreeMap<String, String> = if wire.copy.is_null() {
                BTreeMap::new()
            } else {
                serde_json::from_value(wire.copy).map_err(|_| Unavailable::Unreadable)?
            };
            let poll_seconds = wire.purchase.and_then(|purchase| purchase.checkout).and_then(|checkout| checkout.poll_seconds);
            Ok(Doc::Account(AccountDoc {
                client,
                part: AccountPart { account, blocking: wire.blocking, steps, copy, fallback: wire.fallback, poll_seconds },
                ttl_seconds: wire.ttl_seconds,
            }))
        }
        _ => Err(Unavailable::Unreadable),
    }
}
```

- [ ] **Step 6: Run the tests to see them pass**

Run: the loop command with filter `account_view::doc`.
Expected: `test result: ok. 10 passed; 0 failed`.

- [ ] **Step 7: Mutation-check two tests**

1. Change `if head.schema != SCHEMA_MAJOR` to `if head.schema > 2`. Run. Expected: `a_schema_this_client_does_not_know_is_unknown_schema` fails (left `Err(Unreadable)`, right `Err(UnknownSchema)`). Revert.
2. Change `signup_web_url: wire.signup.and_then(|signup| signup.web_url)` to `signup_web_url: None`. Run. Expected: `pre_account_reads_the_status_the_signup_url_and_the_fallback_and_never_the_steps` fails on `signup_web_url`. Revert.

Paste each failing assertion, then the green rerun, into the Notes.

- [ ] **Step 8: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t2-check.log 2>&1; echo "rc=$?"
cd $WT && git add contracts/onboarding src-tauri/src/account_view/mod.rs src-tauri/src/account_view/doc.rs src-tauri/src/lib.rs
git commit -m "desktop: vendor the onboarding contract and parse it tolerantly" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- contracts/onboarding src-tauri/src/account_view/mod.rs src-tauri/src/account_view/doc.rs src-tauri/src/lib.rs
git show --stat HEAD
```

---

## Task 3: Link checks and resolution (`account_view::links`, spec §5.1)

**Lane R.** Pure.

**Files:**
- Create: `src-tauri/src/account_view/links.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod links;`)

**Interfaces:**
- Consumes: `doc::{AccountPart, Fallback, FallbackKind, PreAccount}` and the test helpers `doc::tests::{edited, fixture}` (Task 2).
- Produces (`crate::account_view::links`):
  - `CREATE_ACCOUNT_URL = "https://app.beebeeb.io/signup"`, `BILLING_URL = "https://app.beebeeb.io/billing"`, `SUPPORT_URL = "https://beebeeb.io/support"` (the only Rust copies of these three)
  - `struct Links { create_account: String, billing: String, support: String, step: String }` (Serialize; the `links` field of `AccountView`), `Links::built_in()`
  - `fn checked(url: &str) -> Option<String>`, `fn checked_with(url: &str, allow_localhost: bool) -> Option<String>`
  - `fn resolve(pre: Option<&PreAccount>, part: Option<&AccountPart>) -> Links`

- [ ] **Step 1: Write `links.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 §5.1: every web link the desktop opens for the account, resolved here. A server
//! link is used only when `checked` accepts it; otherwise its destination's built-in address. The
//! webview's opener allows any http(s) URL (`capabilities/default.json`), so `checked` is the only
//! filter there is. The frontend opens exactly the string it is given.

use reqwest::Url;
use serde::Serialize;

use super::doc::{AccountPart, Fallback, FallbackKind, PreAccount};

pub const CREATE_ACCOUNT_URL: &str = "https://app.beebeeb.io/signup";
pub const BILLING_URL: &str = "https://app.beebeeb.io/billing";
pub const SUPPORT_URL: &str = "https://beebeeb.io/support";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Links {
    /// "Create an account on beebeeb.io": the pre_account `signup.web_url`.
    pub create_account: String,
    /// Web billing: "Choose a plan", "Manage plan", "Update payment details".
    pub billing: String,
    /// "Contact support".
    pub support: String,
    /// The C-R5 button: the first required step's link, else the document's, else web billing.
    pub step: String,
}

impl Links {
    pub fn built_in() -> Self {
        Self {
            create_account: CREATE_ACCOUNT_URL.to_string(),
            billing: BILLING_URL.to_string(),
            support: SUPPORT_URL.to_string(),
            step: BILLING_URL.to_string(),
        }
    }
}

/// A server link, or `None` when it counts as absent: not `https`, no host, a user-info part, or a
/// host other than `beebeeb.io` and its subdomains. Debug builds also take `http://localhost`.
pub fn checked(url: &str) -> Option<String> {
    checked_with(url, cfg!(debug_assertions))
}

pub fn checked_with(url: &str, allow_localhost: bool) -> Option<String> {
    todo!("Task 3 step 3")
}

pub fn resolve(pre: Option<&PreAccount>, part: Option<&AccountPart>) -> Links {
    todo!("Task 3 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::doc::tests::edited;
    use crate::account_view::doc::{self, Doc};
    use serde_json::json;

    fn part_of(bytes: &[u8]) -> AccountPart {
        doc::parse(bytes).unwrap().account_part().cloned().unwrap()
    }

    fn pre_of(bytes: &[u8]) -> doc::PreAccount {
        match doc::parse(bytes).unwrap() {
            Doc::PreAccount(pre) => pre,
            Doc::Account(_) => panic!("not pre_account"),
        }
    }

    #[test]
    fn a_server_link_must_be_https_on_beebeeb_io_without_user_info() {
        for ok in [
            "https://app.beebeeb.io/signup",
            "https://beebeeb.io/support",
            "https://billing.app.beebeeb.io/plans?from=desktop",
            "https://APP.BEEBEEB.IO/signup",
        ] {
            assert!(checked_with(ok, false).is_some(), "{ok}");
        }
        for bad in [
            "http://app.beebeeb.io/signup",
            "https://beebeeb.io.example.com/",
            "https://evilbeebeeb.io/",
            "https://user:pw@app.beebeeb.io/",
            "https://user@app.beebeeb.io/",
            "https://beebeeb.io./x",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "app.beebeeb.io/signup",
            "",
        ] {
            assert_eq!(checked_with(bad, false), None, "{bad}");
        }
    }

    #[test]
    fn localhost_over_http_is_accepted_only_in_debug_builds() {
        assert_eq!(checked_with("http://localhost:5173/signup", true).as_deref(), Some("http://localhost:5173/signup"));
        assert_eq!(checked_with("http://localhost:5173/signup", false), None);
        assert_eq!(checked_with("http://127.0.0.1:5173/signup", true), None);
        assert_eq!(checked_with("http://localhost.example.com/", true), None);
        assert_eq!(checked("http://localhost/x").is_some(), cfg!(debug_assertions));
    }

    #[test]
    fn an_accepted_link_is_the_servers_address_unchanged() {
        assert_eq!(checked("https://app.beebeeb.io/signup").as_deref(), Some("https://app.beebeeb.io/signup"));
        assert_eq!(checked("https://app.beebeeb.io/billing?plan=pro").as_deref(), Some("https://app.beebeeb.io/billing?plan=pro"));
    }

    #[test]
    fn without_server_links_every_destination_is_its_built_in_address() {
        let links = resolve(None, None);
        assert_eq!(links, Links::built_in());
        assert_eq!(links.step, BILLING_URL);
    }

    #[test]
    fn create_account_is_the_pre_account_signup_url_when_it_passes_the_check() {
        let pre = pre_of(&edited("pre_account.desktop.json", |v| v["signup"]["web_url"] = json!("https://app.beebeeb.io/signup?from=desktop")));
        assert_eq!(resolve(Some(&pre), None).create_account, "https://app.beebeeb.io/signup?from=desktop");
        let pre = pre_of(&edited("pre_account.desktop.json", |v| v["signup"]["web_url"] = json!("https://signup.example.com/")));
        assert_eq!(resolve(Some(&pre), None).create_account, CREATE_ACCOUNT_URL);
    }

    #[test]
    fn billing_and_support_are_the_account_fallback_of_their_kind() {
        let web = part_of(&edited("account.lapsed.ios.json", |v| v["fallback"] = json!({ "kind": "use_web", "url": "https://app.beebeeb.io/billing?state=lapsed" })));
        let links = resolve(None, Some(&web));
        assert_eq!(links.billing, "https://app.beebeeb.io/billing?state=lapsed");
        assert_eq!(links.support, SUPPORT_URL);
        let support = part_of(&edited("account.frozen.desktop.json", |v| v["fallback"] = json!({ "kind": "contact_support", "url": "https://beebeeb.io/support/frozen" })));
        let links = resolve(None, Some(&support));
        assert_eq!(links.support, "https://beebeeb.io/support/frozen");
        assert_eq!(links.billing, BILLING_URL);
        let foreign = part_of(&edited("account.lapsed.ios.json", |v| v["fallback"] = json!({ "kind": "use_web", "url": "https://billing.example.com/" })));
        assert_eq!(resolve(None, Some(&foreign)).billing, BILLING_URL);
    }

    /// Plan "Spec issues" 1: the unknown-step fixture is pre_account, so this account-stage document is
    /// derived from the coupon fixture with extra required steps.
    #[test]
    fn the_step_link_is_the_first_required_step_with_a_checked_url() {
        let with_steps = |future_url: &str, document_fallback: serde_json::Value| {
            part_of(&edited("account.needs_plan.web.coupon.json", |v| {
                v["steps"] = json!([
                    { "id": "redeem_coupon", "status": "todo", "required": false, "ui": "action",
                      "fallback": { "kind": "use_web", "url": "https://app.beebeeb.io/coupon" } },
                    { "id": "choose_plan", "status": "todo", "required": true, "ui": "action" },
                    { "id": "future_step", "status": "todo", "required": true, "ui": "action",
                      "fallback": { "kind": "use_web", "url": future_url } },
                    { "id": "later_step", "status": "todo", "required": true, "ui": "action",
                      "fallback": { "kind": "use_web", "url": "https://app.beebeeb.io/later" } }
                ]);
                v["fallback"] = document_fallback;
            }))
        };
        let part = with_steps("https://app.beebeeb.io/future", serde_json::Value::Null);
        assert_eq!(resolve(None, Some(&part)).step, "https://app.beebeeb.io/future", "the first required step with a link wins; optional steps never do");
        let part = with_steps("https://future.example.com/", serde_json::Value::Null);
        assert_eq!(resolve(None, Some(&part)).step, "https://app.beebeeb.io/later", "a link that fails the check counts as absent");

        let bare = |document_fallback: serde_json::Value| {
            part_of(&edited("account.needs_plan.web.coupon.json", |v| {
                v["steps"] = json!([{ "id": "choose_plan", "status": "todo", "required": true, "ui": "action" }]);
                v["fallback"] = document_fallback;
            }))
        };
        let part = bare(json!({ "kind": "use_web", "url": "https://app.beebeeb.io/billing?from=plan" }));
        let links = resolve(None, Some(&part));
        assert_eq!(links.step, "https://app.beebeeb.io/billing?from=plan", "else the document's link");
        let part = bare(serde_json::Value::Null);
        assert_eq!(resolve(None, Some(&part)).step, BILLING_URL, "else web billing");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Add `pub mod links;` to `account_view/mod.rs`. Run: the loop command with filter `account_view::links`.
Expected: `test result: FAILED. 0 passed; 7 failed` (each `not yet implemented: Task 3 step 3`).

- [ ] **Step 3: Implement**

```rust
pub fn checked_with(url: &str, allow_localhost: bool) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let host = parsed.host_str()?;
    let accepted = match parsed.scheme() {
        "https" => host == "beebeeb.io" || host.ends_with(".beebeeb.io"),
        "http" => allow_localhost && host == "localhost",
        _ => false,
    };
    accepted.then(|| parsed.as_str().to_string())
}

pub fn resolve(pre: Option<&PreAccount>, part: Option<&AccountPart>) -> Links {
    let of_kind = |fallback: Option<&Fallback>, kind: FallbackKind| {
        fallback.filter(|f| f.kind == kind).and_then(|f| f.url.as_deref()).and_then(checked)
    };
    let create_account = pre
        .and_then(|pre| pre.signup_web_url.as_deref())
        .and_then(checked)
        .unwrap_or_else(|| CREATE_ACCOUNT_URL.to_string());
    let billing = part
        .and_then(|part| of_kind(part.fallback.as_ref(), FallbackKind::UseWeb))
        .unwrap_or_else(|| BILLING_URL.to_string());
    let support = part
        .and_then(|part| of_kind(part.fallback.as_ref(), FallbackKind::ContactSupport))
        .unwrap_or_else(|| SUPPORT_URL.to_string());
    let step = part.and_then(step_link).unwrap_or_else(|| billing.clone());
    Links { create_account, billing, support, step }
}

/// §5.1, last row: the `fallback.url` of the first required step, in document order, that has one
/// passing the check; else the document's own `fallback.url`.
fn step_link(part: &AccountPart) -> Option<String> {
    part.steps
        .iter()
        .filter(|step| step.required)
        .find_map(|step| step.fallback.as_ref()?.url.as_deref().and_then(checked))
        .or_else(|| part.fallback.as_ref()?.url.as_deref().and_then(checked))
}
```

`https://APP.BEEBEEB.IO/signup` parses to the lower-case host, which `as_str()` returns; every other accepted link above is already in that form, so it is returned unchanged.

- [ ] **Step 4: Run the tests to see them pass**

Expected: `test result: ok. 7 passed; 0 failed`.

- [ ] **Step 5: Mutation-check**

1. Replace `host.ends_with(".beebeeb.io")` with `host.ends_with("beebeeb.io")`. Expected: `a_server_link_must_be_https_on_beebeeb_io_without_user_info` fails on `https://evilbeebeeb.io/`. Revert.
2. Remove `.filter(|step| step.required)`. Expected: `the_step_link_is_the_first_required_step_with_a_checked_url` fails (left `https://app.beebeeb.io/coupon`). Revert.

Paste both failures and the green rerun into the Notes.

- [ ] **Step 6: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t3-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/links.rs src-tauri/src/account_view/mod.rs
git commit -m "desktop: resolve account links from the onboarding document with built-in addresses" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/links.rs src-tauri/src/account_view/mod.rs
git show --stat HEAD
```

---

## Task 4: Ready notices and the day-month date (`account_view::notice`, spec §7.5)

**Lane R.** Pure. Rust decides which notice and formats its dates; the words live in `src/accountViewCopy.ts` (Task 15).

**Files:**
- Create: `src-tauri/src/account_view/notice.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod notice;`)

**Interfaces:**
- Consumes: `doc::AccountPart` (Task 2), `links::Links` (Task 3).
- Produces (`crate::account_view::notice`):
  - `enum NoticeKind { Trial, ReadOnly, ReadOnlyOther, Frozen, PaymentFailed }` (serde snake_case: `trial`, `read_only`, `read_only_other`, `frozen`, `payment_failed`)
  - `enum NoticeLink { ManagePlan, ChoosePlan, ContactSupport, UpdatePaymentDetails }` (serde snake_case)
  - `struct Notice { kind, server_line: Option<String>, trial_ends: Option<String>, read_only_since: Option<String>, data_deletion_at: Option<String>, link: Option<NoticeLink>, url: Option<String> }` (Serialize)
  - `PLAN_REASONS: [&str; 5]`, `fn day_month(at: DateTime<Utc>) -> String`, `fn notice(part: &AccountPart, links: &Links) -> Option<Notice>`

- [ ] **Step 1: Write `notice.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 §7.5: the one-line Ready notice. Rust decides which notice applies, formats its
//! dates and picks its link; the frontend's copy module (`src/accountViewCopy.ts`) owns the words.
//! Server `copy` for the state wins and is shown verbatim, so legal dates always come from the
//! server. A date the server does not send is never invented: its clause is dropped.

use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;

use super::doc::AccountPart;
use super::links::Links;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Trial,
    ReadOnly,
    /// An unknown state whose upload denial is not a plan reason: "Read-only." alone, no link.
    ReadOnlyOther,
    Frozen,
    PaymentFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLink {
    ManagePlan,
    ChoosePlan,
    ContactSupport,
    UpdatePaymentDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub kind: NoticeKind,
    /// The server's `copy` for this state, shown verbatim instead of the built-in line.
    pub server_line: Option<String>,
    pub trial_ends: Option<String>,
    pub read_only_since: Option<String>,
    pub data_deletion_at: Option<String>,
    pub link: Option<NoticeLink>,
    pub url: Option<String>,
}

/// Upload denials that mean "choose a plan" (server `onboarding/derive.rs`, `reason`).
pub const PLAN_REASONS: [&str; 5] = ["plan_required", "billing_read_only", "account_lapsed", "trial_ended", "trial_cancelled_read_only"];

/// `18 Oct`: the server's `onboarding::day_month`, UTC, English month abbreviations.
pub fn day_month(at: DateTime<Utc>) -> String {
    todo!("Task 4 step 3")
}

pub fn notice(part: &AccountPart, links: &Links) -> Option<Notice> {
    todo!("Task 4 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::doc::tests::{account_part, edited};
    use crate::account_view::doc;
    use chrono::TimeZone;
    use serde_json::json;

    fn links() -> Links {
        Links {
            create_account: "https://app.beebeeb.io/signup".into(),
            billing: "https://app.beebeeb.io/billing?resolved".into(),
            support: "https://beebeeb.io/support?resolved".into(),
            step: "https://app.beebeeb.io/billing?resolved".into(),
        }
    }

    fn part_of(bytes: &[u8]) -> AccountPart {
        doc::parse(bytes).unwrap().account_part().cloned().unwrap()
    }

    fn of(name: &str) -> Option<Notice> {
        notice(&account_part(name), &links())
    }

    #[test]
    fn day_month_matches_the_servers_examples() {
        assert_eq!(day_month(Utc.with_ymd_and_hms(2026, 10, 18, 9, 0, 0).unwrap()), "18 Oct");
        assert_eq!(day_month(Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap()), "1 Nov");
        assert_eq!(day_month(Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap()), "31 Dec");
    }

    #[test]
    fn a_trial_names_its_end_date_and_links_to_manage_plan() {
        let n = of("account.trialing.desktop.json").unwrap();
        assert_eq!((n.kind, n.trial_ends.as_deref(), n.server_line.as_deref()), (NoticeKind::Trial, Some("18 Oct"), None));
        assert_eq!((n.link, n.url.as_deref()), (Some(NoticeLink::ManagePlan), Some("https://app.beebeeb.io/billing?resolved")));
    }

    #[test]
    fn the_server_trial_copy_is_shown_verbatim() {
        let part = account_part("account.trialing_no_card.desktop.json");
        let n = notice(&part, &links()).unwrap();
        assert_eq!(n.kind, NoticeKind::Trial);
        assert_eq!(n.server_line.as_deref(), part.copy.get("trial_end_over_allowance").map(String::as_str));
        let no_allowance = part_of(&edited("account.trialing.desktop.json", |v| v["copy"] = json!({ "trial_end_no_allowance": "Your trial ends on 18 Oct." })));
        assert_eq!(notice(&no_allowance, &links()).unwrap().server_line.as_deref(), Some("Your trial ends on 18 Oct."));
    }

    #[test]
    fn a_trial_without_an_end_date_or_server_copy_has_no_notice() {
        let part = part_of(&edited("account.trialing.desktop.json", |v| v["account"]["trial"] = serde_json::Value::Null));
        assert_eq!(notice(&part, &links()), None);
    }

    #[test]
    fn every_read_only_state_takes_its_dates_from_lifecycle_and_links_to_choose_a_plan() {
        let cases = [
            ("account.trial_cancelling.web.json", Some("6 Oct"), Some("1 Nov")),
            ("account.lapsed.ios.json", None, Some("1 Dec")),
            ("account.read_only.web.json", None, None),
        ];
        for (name, since, deletion) in cases {
            let n = of(name).unwrap();
            assert_eq!(n.kind, NoticeKind::ReadOnly, "{name}");
            assert_eq!((n.read_only_since.as_deref(), n.data_deletion_at.as_deref()), (since, deletion), "{name}");
            assert_eq!((n.link, n.url.as_deref()), (Some(NoticeLink::ChoosePlan), Some("https://app.beebeeb.io/billing?resolved")), "{name}");
            assert_eq!(n.server_line, None, "{name}");
        }
        let ended = of("account.trial_ended.ios.json").unwrap();
        assert_eq!((ended.read_only_since.as_deref(), ended.data_deletion_at.as_deref()), (Some("18 Oct"), Some("1 Nov")));
        assert!(ended.server_line.is_some(), "the fixture sends trial_ended_over_allowance");
        let ended_plain = part_of(&edited("account.trial_ended.ios.json", |v| {
            v["copy"] = json!({});
            v["account"].as_object_mut().unwrap().remove("lifecycle");
        }));
        let n = notice(&ended_plain, &links()).unwrap();
        assert_eq!((n.read_only_since, n.data_deletion_at, n.server_line), (None, None, None), "trial_ended with no lifecycle: neither date");
    }

    #[test]
    fn server_copy_wins_over_every_built_in_line_it_covers() {
        let read_only = part_of(&edited("account.read_only.web.json", |v| v["copy"] = json!({ "trial_ended_over_allowance": "Server words." })));
        assert_eq!(notice(&read_only, &links()).unwrap().server_line.as_deref(), Some("Server words."));
        let trial = part_of(&edited("account.trialing.desktop.json", |v| v["copy"] = json!({ "trial_end_over_allowance": "Server trial words." })));
        assert_eq!(notice(&trial, &links()).unwrap().server_line.as_deref(), Some("Server trial words."));
    }

    #[test]
    fn other_copy_keys_never_become_a_notice() {
        let active = part_of(&edited("account.active.web.json", |v| v["copy"] = json!({
            "locale": "en", "region_line": "Stored in the EU.", "plans_managed_on_web": "x", "trial_terms": "y", "future_key": "z"
        })));
        assert_eq!(notice(&active, &links()), None);
        let frozen = part_of(&edited("account.frozen.desktop.json", |v| v["copy"] = json!({ "plans_managed_on_web": "x" })));
        assert_eq!(notice(&frozen, &links()).unwrap().server_line, None);
    }

    #[test]
    fn frozen_links_to_support_and_payment_failed_to_web_billing() {
        let frozen = of("account.frozen.desktop.json").unwrap();
        assert_eq!((frozen.kind, frozen.link, frozen.url.as_deref()), (NoticeKind::Frozen, Some(NoticeLink::ContactSupport), Some("https://beebeeb.io/support?resolved")));
        let past_due = of("account.past_due.web.json").unwrap();
        assert_eq!((past_due.kind, past_due.link, past_due.url.as_deref()), (NoticeKind::PaymentFailed, Some(NoticeLink::UpdatePaymentDetails), Some("https://app.beebeeb.io/billing?resolved")));
    }

    #[test]
    fn active_legacy_free_and_allowance_have_no_notice() {
        for name in ["account.active.web.json", "account.legacy_free.web.json", "account.allowance.desktop.json"] {
            assert_eq!(of(name), None, "{name}");
        }
    }

    #[test]
    fn an_unknown_state_is_decided_from_its_upload_capability() {
        let unknown = |upload: serde_json::Value| {
            part_of(&edited("account.trial_ended.ios.json", |v| {
                v["account"]["state"] = json!("future_state");
                v["account"]["capabilities"]["upload"] = upload;
                v["copy"] = json!({});
            }))
        };
        for reason in PLAN_REASONS {
            let n = notice(&unknown(json!({ "allowed": false, "reason": reason })), &links()).unwrap();
            assert_eq!((n.kind, n.link), (NoticeKind::ReadOnly, Some(NoticeLink::ChoosePlan)), "{reason}");
            assert_eq!(n.data_deletion_at.as_deref(), Some("1 Nov"), "{reason}: the read-only variants keep their dates");
        }
        assert_eq!(notice(&unknown(json!({ "allowed": true })), &links()), None);
    }

    #[test]
    fn an_unknown_state_denied_for_another_reason_says_read_only_alone_with_no_link() {
        let part = part_of(&edited("account.trial_ended.ios.json", |v| {
            v["account"]["state"] = json!("future_state");
            v["account"]["capabilities"]["upload"] = json!({ "allowed": false, "reason": "future_reason" });
        }));
        let n = notice(&part, &links()).unwrap();
        assert_eq!((n.kind, n.link, n.url), (NoticeKind::ReadOnlyOther, None, None));
        assert_eq!((n.read_only_since, n.data_deletion_at, n.server_line), (None, None, None));
    }

    #[test]
    fn the_notice_serializes_as_the_frontends_contract() {
        let n = of("account.trialing.desktop.json").unwrap();
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            json!({ "kind": "trial", "server_line": null, "trial_ends": "18 Oct", "read_only_since": null,
                    "data_deletion_at": null, "link": "manage_plan", "url": "https://app.beebeeb.io/billing?resolved" })
        );
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Add `pub mod notice;` to `mod.rs`. Run with filter `account_view::notice`.
Expected: `test result: FAILED. 0 passed; 12 failed`.

- [ ] **Step 3: Implement**

```rust
pub fn day_month(at: DateTime<Utc>) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {}", at.day(), MONTHS[at.month0() as usize])
}

pub fn notice(part: &AccountPart, links: &Links) -> Option<Notice> {
    let copy = |key: &str| part.copy.get(key).cloned();
    match part.account.state.as_str() {
        "trialing" | "trialing_no_card" => {
            let server_line = copy("trial_end_over_allowance").or_else(|| copy("trial_end_no_allowance"));
            let trial_ends = part.account.trial.as_ref().map(|trial| day_month(trial.ends_at));
            if server_line.is_none() && trial_ends.is_none() {
                return None;
            }
            Some(Notice {
                trial_ends,
                server_line,
                link: Some(NoticeLink::ManagePlan),
                url: Some(links.billing.clone()),
                ..blank(NoticeKind::Trial)
            })
        }
        "trial_ended" | "trial_cancelling" | "lapsed" | "read_only" => Some(read_only(part, links)),
        "frozen" => Some(Notice { link: Some(NoticeLink::ContactSupport), url: Some(links.support.clone()), ..blank(NoticeKind::Frozen) }),
        "past_due" => Some(Notice {
            link: Some(NoticeLink::UpdatePaymentDetails),
            url: Some(links.billing.clone()),
            ..blank(NoticeKind::PaymentFailed)
        }),
        "active" | "legacy_free" | "allowance" | "needs_plan" => None,
        _ => match part.account.capabilities.upload.as_ref() {
            Some(upload) if upload.allowed => None,
            Some(upload) if upload.reason.as_deref().is_some_and(|reason| PLAN_REASONS.contains(&reason)) => Some(read_only(part, links)),
            _ => Some(blank(NoticeKind::ReadOnlyOther)),
        },
    }
}

fn read_only(part: &AccountPart, links: &Links) -> Notice {
    let lifecycle = part.account.lifecycle.as_ref();
    Notice {
        server_line: part.copy.get("trial_ended_over_allowance").cloned(),
        read_only_since: lifecycle.and_then(|l| l.read_only_since).map(day_month),
        data_deletion_at: lifecycle.and_then(|l| l.data_deletion_at).map(day_month),
        link: Some(NoticeLink::ChoosePlan),
        url: Some(links.billing.clone()),
        ..blank(NoticeKind::ReadOnly)
    }
}

fn blank(kind: NoticeKind) -> Notice {
    Notice { kind, server_line: None, trial_ends: None, read_only_since: None, data_deletion_at: None, link: None, url: None }
}
```

An absent `upload` capability (`None`) falls to the last arm: "not allowed" with no reason, so "Read-only." alone.

- [ ] **Step 4: Run the tests to see them pass**

Expected: `test result: ok. 12 passed; 0 failed`.

- [ ] **Step 5: Mutation-check**

1. In `read_only`, replace the `read_only_since` line with `read_only_since: lifecycle.and_then(|l| l.data_deletion_at).map(day_month),`. Expected: `every_read_only_state_takes_its_dates_from_lifecycle_and_links_to_choose_a_plan` fails on its first case, `account.trial_cancelling.web.json` (left `(Some("1 Nov"), Some("1 Nov"))`, right `(Some("6 Oct"), Some("1 Nov"))`). Revert.
2. Move `"past_due"` into the no-notice arm. Expected: `frozen_links_to_support_and_payment_failed_to_web_billing` fails (`called Option::unwrap() on a None value`). Revert.

Paste both and the green rerun.

- [ ] **Step 6: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t4-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/notice.rs src-tauri/src/account_view/mod.rs
git commit -m "desktop: derive the account notice and its dates from the onboarding document" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/notice.rs src-tauri/src/account_view/mod.rs
git show --stat HEAD
```

---

## Task 5: The account state — the §4.2 table, C-R8's precedence and the gate value (`account_view::derive`)

**Lane R.** Pure. This is the function the spec's test list calls "the table": tested row by row, on every precedence overlap, and mutation-checked.

**Files:**
- Create: `src-tauri/src/account_view/derive.rs`
- Create: `src-tauri/tests/fixtures/account-view/signed_out.json`, `ready_trial.json`, `no_plan_offline.json`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod derive;`)

**Interfaces:**
- Consumes: `doc::{AccountPart, ClientStatus, Doc}` (Task 2), `links::{self, Links}` (Task 3), `notice::{self, Notice}` (Task 4), `crate::surfaces::policy::Platform` (spec A).
- Produces (`crate::account_view::derive`):
  - `enum TokenState { Absent, Restoring, Stored }`, `enum FirstFetch { Pending, Concluded }`
  - `enum SessionKind { None, Valid, Ended }` (serde `none`, `valid`, `ended`)
  - `enum AccountState { Checking, SignedOut, SessionEnded, UpdateRequired, Locked, NoPlan, Ready }` (serde snake_case), `AccountState::ALL`, `fn as_str(self) -> &'static str`
  - `enum Screen { SignIn, SignInAgain, Update, RecoveryPhrase, KeychainUnlock, ChoosePlan, Busy, Nothing }` (serde snake_case)
  - `enum Blocking { None, NeedsPlan, VerifyEmail, UnknownStep }`
  - `struct Inputs<'a> { platform: Platform, token: TokenState, auth_expired: bool, owner_recorded: bool, key_in_memory: bool, key_in_keychain: bool, fresh: Option<&'a Doc>, cached: Option<&'a AccountPart>, first_fetch: FirstFetch, offline: bool }`
  - `struct AccountView { state, session, offline: bool, screen, busy: bool, links: Links, notice: Option<Notice> }` (Serialize: the frontend's contract), `AccountView::signed_out(offline: bool) -> AccountView`
  - `struct Derived { view: AccountView, gate_open: bool, blocking: Blocking, condition: AccountState }` (`condition`: the state with the key assumed in memory; the gate and the driver's "why held" follow it)
  - `fn session_kind(&Inputs) -> SessionKind`, `fn derive(&Inputs) -> Derived`

The JSON the frontend reads (Task 15 parses exactly this):

```json
{ "state": "ready", "session": "valid", "offline": false, "screen": "nothing", "busy": false,
  "links": { "create_account": "https://app.beebeeb.io/signup", "billing": "https://app.beebeeb.io/billing",
             "support": "https://beebeeb.io/support", "step": "https://app.beebeeb.io/billing" },
  "notice": { "kind": "trial", "server_line": null, "trial_ends": "18 Oct", "read_only_since": null,
              "data_deletion_at": null, "link": "manage_plan", "url": "https://app.beebeeb.io/billing" } }
```

- [ ] **Step 1: Write the three contract fixtures**

`src-tauri/tests/fixtures/account-view/signed_out.json`:

```json
{
  "state": "signed_out",
  "session": "none",
  "offline": false,
  "screen": "sign_in",
  "busy": false,
  "links": {
    "create_account": "https://app.beebeeb.io/signup",
    "billing": "https://app.beebeeb.io/billing",
    "support": "https://beebeeb.io/support",
    "step": "https://app.beebeeb.io/billing"
  },
  "notice": null
}
```

`src-tauri/tests/fixtures/account-view/ready_trial.json`:

```json
{
  "state": "ready",
  "session": "valid",
  "offline": false,
  "screen": "nothing",
  "busy": false,
  "links": {
    "create_account": "https://app.beebeeb.io/signup",
    "billing": "https://app.beebeeb.io/billing",
    "support": "https://beebeeb.io/support",
    "step": "https://app.beebeeb.io/billing"
  },
  "notice": {
    "kind": "trial",
    "server_line": null,
    "trial_ends": "18 Oct",
    "read_only_since": null,
    "data_deletion_at": null,
    "link": "manage_plan",
    "url": "https://app.beebeeb.io/billing"
  }
}
```

`src-tauri/tests/fixtures/account-view/no_plan_offline.json`:

```json
{
  "state": "no_plan",
  "session": "valid",
  "offline": true,
  "screen": "choose_plan",
  "busy": false,
  "links": {
    "create_account": "https://app.beebeeb.io/signup",
    "billing": "https://app.beebeeb.io/billing",
    "support": "https://beebeeb.io/support",
    "step": "https://app.beebeeb.io/billing"
  },
  "notice": null
}
```

- [ ] **Step 2: Write `derive.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 §4: one account state from the session, the keys, the onboarding document and
//! connectivity. The §4.2 table and C-R8's precedence are `derive`, one pure function; every window
//! and the menu-bar icon render what it returns. The gate value (C-R10) is decided here too, from
//! the account's condition with the key assumed present (plan "Spec issues" 3): an unlock can then
//! never start an engine for an account that is still being checked, needs an update or has no plan.

use serde::Serialize;

use super::doc::{AccountPart, ClientStatus, Doc};
use super::links::{self, Links};
use super::notice::{self, Notice};
use crate::surfaces::policy::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// No usable session token on this computer.
    Absent,
    /// A stored token whose startup restore and probe are still running (C-R11, first entry). Not a
    /// "valid session" for C-R4 (plan "Spec issues" 4).
    Restoring,
    /// A stored token whose restore has finished.
    Stored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstFetch {
    /// The first fetch since this session began has not concluded.
    Pending,
    /// It concluded, with a fresh document or "unavailable" of any kind, a network failure included.
    Concluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    None,
    Valid,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Checking,
    SignedOut,
    SessionEnded,
    UpdateRequired,
    Locked,
    NoPlan,
    Ready,
}

impl AccountState {
    pub const ALL: [Self; 7] = [Self::Checking, Self::SignedOut, Self::SessionEnded, Self::UpdateRequired, Self::Locked, Self::NoPlan, Self::Ready];

    /// The lifecycle log's `<s>` (§10).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::SignedOut => "signed_out",
            Self::SessionEnded => "session_ended",
            Self::UpdateRequired => "update_required",
            Self::Locked => "locked",
            Self::NoPlan => "no_plan",
            Self::Ready => "ready",
        }
    }
}

/// Which screen the account window shows (C-W3). `KeychainUnlock` is the Settings window's Unlock
/// (C-W7); `Nothing` is Ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    SignIn,
    SignInAgain,
    Update,
    RecoveryPhrase,
    KeychainUnlock,
    ChoosePlan,
    Busy,
    Nothing,
}

/// Why a document is blocking. C-R15: only `NeedsPlan` gets a screen; the others proceed as today
/// with one lifecycle line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocking {
    None,
    NeedsPlan,
    VerifyEmail,
    UnknownStep,
}

#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    pub platform: Platform,
    pub token: TokenState,
    /// Spec A's `AuthHealth::is_expired()`.
    pub auth_expired: bool,
    /// Spec A's owner record (`StateDb::owner()` is `Some`). An unreadable record counts as recorded.
    pub owner_recorded: bool,
    pub key_in_memory: bool,
    /// Spec A's `keychain_vault_key_present` (always `false` on Windows).
    pub key_in_keychain: bool,
    /// The latest document of this session generation, kept across later failed fetches.
    pub fresh: Option<&'a Doc>,
    /// The cached account part, only when it is the same account (C-D5).
    pub cached: Option<&'a AccountPart>,
    pub first_fetch: FirstFetch,
    pub offline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountView {
    pub state: AccountState,
    pub session: SessionKind,
    pub offline: bool,
    pub screen: Screen,
    pub busy: bool,
    pub links: Links,
    pub notice: Option<Notice>,
}

impl AccountView {
    /// What a sign-out by choice or an account switch leaves, set inside that transition (C-D5).
    pub fn signed_out(offline: bool) -> Self {
        Self {
            state: AccountState::SignedOut,
            session: SessionKind::None,
            offline,
            screen: Screen::SignIn,
            busy: false,
            links: Links::built_in(),
            notice: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub view: AccountView,
    pub gate_open: bool,
    pub blocking: Blocking,
    /// The state with the key assumed in memory: what the gate follows, and why the driver says the engine is held.
    pub condition: AccountState,
}

pub fn session_kind(i: &Inputs<'_>) -> SessionKind {
    todo!("Task 5 step 4")
}

pub fn derive(i: &Inputs<'_>) -> Derived {
    todo!("Task 5 step 4")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::doc::tests::{edited, fixture};
    use crate::account_view::doc::parse;
    use serde_json::json;
    use AccountState::*;

    fn doc(bytes: &[u8]) -> &'static Doc {
        Box::leak(Box::new(parse(bytes).unwrap()))
    }

    fn named(name: &str) -> &'static Doc {
        doc(&fixture(name))
    }

    fn part(name: &str) -> &'static AccountPart {
        named(name).account_part().unwrap()
    }

    fn update_required_account() -> &'static Doc {
        doc(&edited("account.active.web.json", |v| v["client"]["status"] = json!("update_required")))
    }

    /// Signed in on a Mac, key in memory, restore done, first fetch concluded with nothing.
    fn base() -> Inputs<'static> {
        Inputs {
            platform: Platform::Macos,
            token: TokenState::Stored,
            auth_expired: false,
            owner_recorded: true,
            key_in_memory: true,
            key_in_keychain: true,
            fresh: None,
            cached: None,
            first_fetch: FirstFetch::Concluded,
            offline: false,
        }
    }

    fn state(i: Inputs<'static>) -> AccountState {
        derive(&i).view.state
    }

    #[test]
    fn signed_out_needs_no_token_and_no_recorded_owner() {
        let d = derive(&Inputs { token: TokenState::Absent, owner_recorded: false, key_in_memory: false, ..base() });
        assert_eq!((d.view.state, d.view.session, d.view.screen), (SignedOut, SessionKind::None, Screen::SignIn));
        assert!(d.gate_open);
    }

    #[test]
    fn session_ended_from_the_auth_expired_state() {
        let d = derive(&Inputs { auth_expired: true, ..base() });
        assert_eq!((d.view.state, d.view.session, d.view.screen), (SessionEnded, SessionKind::Ended, Screen::SignInAgain));
        assert!(d.gate_open, "spec A's handling is unchanged: the gate does not hold an ended session");
    }

    /// Spec §14: after a startup 401 that kept the owner (whether or not local data remains: the owner
    /// record alone decides), the relaunch derives Session ended; after a sign-out by choice, which
    /// purges the owner record, it derives Signed out.
    #[test]
    fn a_missing_token_beside_a_recorded_owner_is_session_ended_and_without_one_signed_out() {
        assert_eq!(state(Inputs { token: TokenState::Absent, key_in_memory: false, owner_recorded: true, ..base() }), SessionEnded);
        assert_eq!(state(Inputs { token: TokenState::Absent, key_in_memory: false, owner_recorded: false, ..base() }), SignedOut);
    }

    #[test]
    fn update_required_comes_only_from_a_fresh_document() {
        let cached = part("account.active.web.json");
        assert_eq!(state(Inputs { cached: Some(cached), ..base() }), Ready);
        assert_eq!(state(Inputs { fresh: Some(update_required_account()), ..base() }), UpdateRequired);
        let signed_out = Inputs { token: TokenState::Absent, owner_recorded: false, key_in_memory: false, ..base() };
        assert_eq!(state(Inputs { fresh: Some(named("client.update_required.ios.json")), ..signed_out }), UpdateRequired);
    }

    #[test]
    fn update_required_outranks_every_other_state_and_holds_only_a_valid_session() {
        let fresh = Some(update_required_account());
        for (label, inputs, gate_open) in [
            ("signed out", Inputs { token: TokenState::Absent, owner_recorded: false, key_in_memory: false, ..base() }, true),
            ("session ended", Inputs { auth_expired: true, ..base() }, true),
            ("locked", Inputs { key_in_memory: false, ..base() }, false),
            ("restoring", Inputs { token: TokenState::Restoring, key_in_memory: false, ..base() }, false),
            ("no plan", Inputs { cached: Some(part("account.needs_plan.ios.json")), ..base() }, false),
        ] {
            let d = derive(&Inputs { fresh, ..inputs });
            assert_eq!((d.view.state, d.view.screen), (UpdateRequired, Screen::Update), "{label}");
            assert_eq!(d.gate_open, gate_open, "{label}");
        }
    }

    #[test]
    fn locked_is_a_valid_session_without_the_key_in_memory() {
        let d = derive(&Inputs { key_in_memory: false, key_in_keychain: true, ..base() });
        assert_eq!((d.view.state, d.view.screen), (Locked, Screen::KeychainUnlock));
        let d = derive(&Inputs { key_in_memory: false, key_in_keychain: false, ..base() });
        assert_eq!((d.view.state, d.view.screen), (Locked, Screen::RecoveryPhrase));
    }

    #[test]
    fn checking_covers_the_startup_restore() {
        for platform in [Platform::Macos, Platform::Windows] {
            let d = derive(&Inputs { platform, token: TokenState::Restoring, key_in_memory: false, ..base() });
            assert_eq!((d.view.state, d.view.screen, d.view.busy), (Checking, Screen::Busy, true), "{platform:?}");
            assert!(!d.gate_open, "{platform:?}");
        }
    }

    /// Spec §14 (C-S1): a browser sign-in that brought its keys goes to Checking and then on, never to
    /// the recovery phrase. This view decides it, not the frontend's `vaultUnlocked` (Task 17's macOS
    /// window renders only this view).
    #[test]
    fn a_browser_sign_in_with_its_keys_is_never_the_recovery_phrase() {
        let signed_in = Inputs { key_in_memory: true, key_in_keychain: false, owner_recorded: false, first_fetch: FirstFetch::Pending, ..base() };
        let d = derive(&signed_in);
        assert_eq!((d.view.state, d.view.screen), (Checking, Screen::Busy));
        for fresh in [None, Some(named("account.active.web.json")), Some(named("account.needs_plan.ios.json"))] {
            let d = derive(&Inputs { fresh, first_fetch: FirstFetch::Concluded, ..signed_in });
            assert_ne!(d.view.screen, Screen::RecoveryPhrase, "{:?}", d.view.state);
        }
    }

    /// C-R11's other entries: a sign-in or an unlock leaves the key in memory with no cached part and
    /// the first fetch pending (the same inputs, whichever of them got there).
    #[test]
    fn checking_waits_for_the_first_fetch_when_there_is_no_cached_part() {
        let d = derive(&Inputs { first_fetch: FirstFetch::Pending, ..base() });
        assert_eq!((d.view.state, d.view.screen, d.view.busy), (Checking, Screen::Busy, true));
        assert!(!d.gate_open);
    }

    #[test]
    fn checking_ends_on_a_fresh_or_an_unavailable_document() {
        assert_eq!(state(Inputs { first_fetch: FirstFetch::Concluded, ..base() }), Ready, "unavailable: proceed as today");
        assert!(derive(&Inputs { first_fetch: FirstFetch::Concluded, ..base() }).gate_open);
        assert_eq!(state(Inputs { first_fetch: FirstFetch::Pending, fresh: Some(named("account.active.web.json")), ..base() }), Ready);
    }

    #[test]
    fn the_cache_decides_while_the_first_fetch_is_pending_or_unavailable() {
        let cached = Some(part("account.needs_plan.ios.json"));
        assert_eq!(state(Inputs { first_fetch: FirstFetch::Pending, cached, ..base() }), NoPlan);
        assert_eq!(state(Inputs { first_fetch: FirstFetch::Concluded, cached, ..base() }), NoPlan);
        let fresh_ready = Some(named("account.active.web.json"));
        assert_eq!(state(Inputs { fresh: fresh_ready, cached, ..base() }), Ready, "a fresh document decides over the cache");
    }

    #[test]
    fn no_plan_is_a_blocking_needs_plan_document() {
        let d = derive(&Inputs { fresh: Some(named("account.needs_plan.ios.json")), ..base() });
        assert_eq!((d.view.state, d.view.screen, d.blocking), (NoPlan, Screen::ChoosePlan, Blocking::NeedsPlan));
        assert!(!d.gate_open);
    }

    #[test]
    fn blocking_without_needs_plan_proceeds_as_ready_and_names_why() {
        let verify = doc(&edited("account.active.web.json", |v| {
            v["blocking"] = json!(true);
            v["steps"] = json!([{ "id": "verify_email", "status": "todo", "required": true, "ui": "action" }]);
        }));
        let d = derive(&Inputs { fresh: Some(verify), ..base() });
        assert_eq!((d.view.state, d.blocking, d.gate_open), (Ready, Blocking::VerifyEmail, true));
        let unknown = doc(&edited("account.active.web.json", |v| {
            v["blocking"] = json!(true);
            v["steps"] = json!([{ "id": "future_step", "status": "todo", "required": true, "ui": "action" }]);
        }));
        let d = derive(&Inputs { fresh: Some(unknown), ..base() });
        assert_eq!((d.view.state, d.blocking, d.gate_open), (Ready, Blocking::UnknownStep, true));
    }

    #[test]
    fn ready_otherwise() {
        let d = derive(&Inputs { fresh: Some(named("account.active.web.json")), ..base() });
        assert_eq!((d.view.state, d.view.screen, d.view.busy, d.blocking), (Ready, Screen::Nothing, false, Blocking::None));
        assert!(d.gate_open);
    }

    /// C-R8: the first matching row wins, in the order C-R3, C-R2, C-R1, C-R4, Checking, C-R5, C-R6.
    #[test]
    fn precedence_follows_c_r8() {
        let needs_plan = Some(named("account.needs_plan.ios.json"));
        let cases: [(&str, Inputs<'static>, AccountState); 6] = [
            ("C-R2 over C-R4", Inputs { auth_expired: true, key_in_memory: false, ..base() }, SessionEnded),
            ("C-R2 over C-R1", Inputs { token: TokenState::Absent, owner_recorded: true, key_in_memory: false, ..base() }, SessionEnded),
            ("C-R4 over C-R5", Inputs { key_in_memory: false, fresh: needs_plan, ..base() }, Locked),
            ("C-R4 over Checking", Inputs { key_in_memory: false, first_fetch: FirstFetch::Pending, ..base() }, Locked),
            ("Checking over C-R5 and C-R6", Inputs { token: TokenState::Restoring, key_in_memory: false, cached: Some(part("account.needs_plan.ios.json")), ..base() }, Checking),
            ("C-R5 over C-R6", Inputs { fresh: needs_plan, ..base() }, NoPlan),
        ];
        for (label, inputs, expected) in cases {
            assert_eq!(state(inputs), expected, "{label}");
        }
    }

    #[test]
    fn the_gate_is_closed_in_checking_update_required_and_no_plan_while_the_session_is_valid() {
        for platform in [Platform::Macos, Platform::Windows] {
            for (label, inputs, open) in [
                ("checking", Inputs { platform, first_fetch: FirstFetch::Pending, ..base() }, false),
                ("update required", Inputs { platform, fresh: Some(update_required_account()), ..base() }, false),
                ("no plan", Inputs { platform, fresh: Some(named("account.needs_plan.ios.json")), ..base() }, false),
                ("ready", Inputs { platform, ..base() }, true),
                ("signed out", Inputs { platform, token: TokenState::Absent, owner_recorded: false, key_in_memory: false, ..base() }, true),
                ("session ended", Inputs { platform, auth_expired: true, ..base() }, true),
            ] {
                assert_eq!(derive(&inputs).gate_open, open, "{platform:?} {label}");
            }
        }
    }

    #[test]
    fn the_gate_never_holds_on_linux() {
        for inputs in [
            Inputs { platform: Platform::Linux, first_fetch: FirstFetch::Pending, ..base() },
            Inputs { platform: Platform::Linux, token: TokenState::Restoring, key_in_memory: false, ..base() },
            Inputs { platform: Platform::Linux, fresh: Some(update_required_account()), ..base() },
            Inputs { platform: Platform::Linux, fresh: Some(named("account.needs_plan.ios.json")), ..base() },
        ] {
            let d = derive(&inputs);
            assert!(d.gate_open, "{:?}", d.view.state);
        }
        assert_eq!(state(Inputs { platform: Platform::Linux, fresh: Some(named("account.needs_plan.ios.json")), ..base() }), NoPlan, "the state is the same; only the gate differs");
    }

    /// Review Focus 1, plan "Spec issues" 3.
    #[test]
    fn the_gate_follows_the_account_not_the_key() {
        let locked = Inputs { key_in_memory: false, ..base() };
        let d = derive(&Inputs { cached: Some(part("account.needs_plan.ios.json")), ..locked });
        assert_eq!((d.view.state, d.gate_open), (Locked, false), "an unlock must find the gate already closed");
        assert_eq!(d.condition, NoPlan, "held because the account has no plan");
        let d = derive(&Inputs { cached: Some(part("account.active.web.json")), ..locked });
        assert_eq!((d.view.state, d.gate_open), (Locked, true));
        let d = derive(&Inputs { first_fetch: FirstFetch::Pending, ..locked });
        assert_eq!((d.view.state, d.gate_open), (Locked, false), "no cached part: the unlock leads to Checking");
    }

    /// Review Focus 4: C-R13, a cached blocking part keeps its gate until a fresh document decides.
    #[test]
    fn offline_relaunch_with_a_cached_no_plan_part_keeps_choose_a_plan() {
        let d = derive(&Inputs { cached: Some(part("account.needs_plan.ios.json")), first_fetch: FirstFetch::Concluded, offline: true, ..base() });
        assert_eq!((d.view.state, d.view.screen, d.view.offline), (NoPlan, Screen::ChoosePlan, true));
        assert!(!d.gate_open);
    }

    #[test]
    fn offline_is_an_overlay_on_every_state() {
        for inputs in [
            Inputs { token: TokenState::Absent, owner_recorded: false, key_in_memory: false, ..base() },
            Inputs { auth_expired: true, ..base() },
            Inputs { key_in_memory: false, ..base() },
            Inputs { first_fetch: FirstFetch::Pending, ..base() },
            Inputs { fresh: Some(named("account.needs_plan.ios.json")), ..base() },
            base(),
        ] {
            let online = derive(&inputs);
            let offline = derive(&Inputs { offline: true, ..inputs });
            assert!(offline.view.offline && !online.view.offline);
            assert_eq!((offline.view.state, offline.view.screen, offline.gate_open), (online.view.state, online.view.screen, online.gate_open));
        }
    }

    #[test]
    fn only_ready_carries_a_notice() {
        let trial = Some(named("account.trialing.desktop.json"));
        assert!(derive(&Inputs { fresh: trial, ..base() }).view.notice.is_some());
        assert_eq!(derive(&Inputs { fresh: trial, key_in_memory: false, ..base() }).view.notice, None);
    }

    #[test]
    fn the_view_serializes_as_the_frontends_contract() {
        let file = |name: &str| -> serde_json::Value {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/account-view").join(name);
            serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
        };
        assert_eq!(serde_json::to_value(AccountView::signed_out(false)).unwrap(), file("signed_out.json"));
        let ready = derive(&Inputs { fresh: Some(named("account.trialing.desktop.json")), ..base() }).view;
        assert_eq!(serde_json::to_value(ready).unwrap(), file("ready_trial.json"));
        let no_plan = derive(&Inputs { cached: Some(part("account.needs_plan.ios.json")), offline: true, ..base() }).view;
        assert_eq!(serde_json::to_value(no_plan).unwrap(), file("no_plan_offline.json"));
        let names: Vec<&str> = AccountState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["checking", "signed_out", "session_ended", "update_required", "locked", "no_plan", "ready"]);
        for s in AccountState::ALL {
            assert_eq!(serde_json::to_value(s).unwrap(), json!(s.as_str()));
        }
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Add `pub mod derive;` to `mod.rs`. Run with filter `account_view::derive`.
Expected: `test result: FAILED. 0 passed; 22 failed` (each `not yet implemented: Task 5 step 4`).

- [ ] **Step 4: Implement**

```rust
pub fn session_kind(i: &Inputs<'_>) -> SessionKind {
    match i.token {
        TokenState::Absent if i.owner_recorded => SessionKind::Ended,
        TokenState::Absent => SessionKind::None,
        TokenState::Restoring | TokenState::Stored if i.auth_expired => SessionKind::Ended,
        TokenState::Restoring | TokenState::Stored => SessionKind::Valid,
    }
}

pub fn derive(i: &Inputs<'_>) -> Derived {
    let session = session_kind(i);
    let part = i.fresh.and_then(Doc::account_part).or(i.cached);
    let blocking = blocking_of(part);
    let condition = condition(i, session, blocking, part.is_some());
    // C-R4 sits between C-R1 and Checking in C-R8's order. During the restore the token is not yet a
    // valid session (plan "Spec issues" 4), so the restore stays Checking.
    let state = match condition {
        AccountState::Checking | AccountState::NoPlan | AccountState::Ready if i.token == TokenState::Stored && !i.key_in_memory => {
            AccountState::Locked
        }
        other => other,
    };
    let gate_open = i.platform == Platform::Linux
        || !(session == SessionKind::Valid
            && matches!(condition, AccountState::Checking | AccountState::UpdateRequired | AccountState::NoPlan));
    let screen = match state {
        AccountState::SignedOut => Screen::SignIn,
        AccountState::SessionEnded => Screen::SignInAgain,
        AccountState::UpdateRequired => Screen::Update,
        AccountState::Locked if i.key_in_keychain => Screen::KeychainUnlock,
        AccountState::Locked => Screen::RecoveryPhrase,
        AccountState::Checking => Screen::Busy,
        AccountState::NoPlan => Screen::ChoosePlan,
        AccountState::Ready => Screen::Nothing,
    };
    let links = links::resolve(i.fresh.and_then(Doc::pre_account), part);
    let notice = if state == AccountState::Ready { part.and_then(|part| notice::notice(part, &links)) } else { None };
    Derived {
        view: AccountView { state, session, offline: i.offline, screen, busy: state == AccountState::Checking, links, notice },
        gate_open,
        blocking,
        condition,
    }
}

/// The state the account would be in with its key in memory (what the gate follows).
fn condition(i: &Inputs<'_>, session: SessionKind, blocking: Blocking, has_part: bool) -> AccountState {
    if i.fresh.is_some_and(|doc| doc.client() == ClientStatus::UpdateRequired) {
        return AccountState::UpdateRequired;
    }
    match session {
        SessionKind::Ended => return AccountState::SessionEnded,
        SessionKind::None => return AccountState::SignedOut,
        SessionKind::Valid => {}
    }
    if i.token == TokenState::Restoring || (!has_part && i.first_fetch == FirstFetch::Pending) {
        return AccountState::Checking;
    }
    if blocking == Blocking::NeedsPlan {
        return AccountState::NoPlan;
    }
    AccountState::Ready
}

fn blocking_of(part: Option<&AccountPart>) -> Blocking {
    let Some(part) = part.filter(|part| part.blocking) else {
        return Blocking::None;
    };
    if part.account.state == "needs_plan" {
        return Blocking::NeedsPlan;
    }
    let open: Vec<&str> = part.steps.iter().filter(|s| s.required && s.status != "done").map(|s| s.id.as_str()).collect();
    if !open.is_empty() && open.iter().all(|id| *id == "verify_email") {
        Blocking::VerifyEmail
    } else {
        Blocking::UnknownStep
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Expected: `test result: ok. 22 passed; 0 failed`.

- [ ] **Step 6: Mutation-check the table (spec §14)**

Flip one row at a time, run, paste the failing test names and assertions, revert:
1. `TokenState::Absent if i.owner_recorded => SessionKind::Ended` → `SessionKind::None`. Expected failures: `a_missing_token_beside_a_recorded_owner_is_session_ended_and_without_one_signed_out`, `precedence_follows_c_r8` ("C-R2 over C-R1").
2. In `condition`, move the `UpdateRequired` check below the `match session`. Expected: `update_required_outranks_every_other_state_and_holds_only_a_valid_session` ("signed out").
3. Remove `i.token == TokenState::Restoring ||`. Expected: `checking_covers_the_startup_restore` and `precedence_follows_c_r8` ("Checking over C-R5 and C-R6": left `NoPlan`).
4. In `gate_open`, use `state` instead of `condition`. Expected: `the_gate_follows_the_account_not_the_key`.
5. Remove `i.platform == Platform::Linux ||`. Expected: `the_gate_never_holds_on_linux`.
6. In `derive`'s `state` match, replace `!i.key_in_memory` with `!i.key_in_keychain`. Expected: `a_browser_sign_in_with_its_keys_is_never_the_recovery_phrase` (left `(Locked, RecoveryPhrase)`, right `(Checking, Busy)`) and `locked_is_a_valid_session_without_the_key_in_memory`.

- [ ] **Step 7: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t5-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/derive.rs src-tauri/src/account_view/mod.rs src-tauri/tests/fixtures/account-view
git commit -m "desktop: derive one account state and the account gate from session, keys, document and connectivity" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/derive.rs src-tauri/src/account_view/mod.rs src-tauri/tests/fixtures/account-view
git show --stat HEAD
```

---

## Task 6: The fetch schedule (`account_view::policy`, spec C-D4, §10)

**Lane R.** Pure; every time is passed in, so the whole schedule runs on a fake clock.

**Files:**
- Create: `src-tauri/src/account_view/policy.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod policy;`)

**Interfaces:**
- Consumes: `doc::Stage` (Task 2).
- Produces (`crate::account_view::policy`):
  - constants `ACCOUNT_TTL` (60 s), `PRE_ACCOUNT_TTL` (300 s), `PLAN_POLL` (3 s), `TTL_MIN`/`TTL_MAX` (30/900 s), `POLL_MIN`/`POLL_MAX` (2/30 s), `FIRST_FAILURE_RETRY` (10 s), `OFFLINE_MIN_GAP` (10 s), `OFFLINE_PROBE` (30 s), `PLAN_POLL_WINDOW` (15 min), `RETRY_AFTER_CAP` (300 s), `LAUNCH_SETTLE` (5 s), `CHECKING_WATCHDOG` (15 s)
  - `fn ttl(server: Option<u64>, stage: Stage) -> Duration`, `fn poll(server: Option<u64>) -> Duration`
  - `enum Outcome { Document, Answered, RateLimited(Option<Duration>), Network }`
  - `struct Schedule` (Default) with `offline()`, `last_attempt()`, `record(outcome, now, ttl) -> bool` (Offline changed), `plan_opened(now)`, `plan_polling(now) -> bool`, `next_fetch_at(now, ttl, poll) -> Instant`, `focus_wants_fetch(now, poll) -> bool`
  - `fn settle_deadline(probe_done: Instant) -> Instant`, `fn checking_deadline(since: Instant) -> Instant` (no wake function: Spec issue 21)

- [ ] **Step 1: Write `policy.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 C-D4 and §10: when the onboarding document is fetched, and when Beebeeb counts
//! as offline. The schedule lives here and nowhere else; the clock itself is passed in (`now`), so
//! every rule runs on a fake clock in the tests.

use std::time::{Duration, Instant};

use super::doc::Stage;

pub const ACCOUNT_TTL: Duration = Duration::from_secs(60);
pub const PRE_ACCOUNT_TTL: Duration = Duration::from_secs(300);
pub const PLAN_POLL: Duration = Duration::from_secs(3);
pub const TTL_MIN: Duration = Duration::from_secs(30);
pub const TTL_MAX: Duration = Duration::from_secs(900);
pub const POLL_MIN: Duration = Duration::from_secs(2);
pub const POLL_MAX: Duration = Duration::from_secs(30);
pub const FIRST_FAILURE_RETRY: Duration = Duration::from_secs(10);
pub const OFFLINE_MIN_GAP: Duration = Duration::from_secs(10);
pub const OFFLINE_PROBE: Duration = Duration::from_secs(30);
pub const PLAN_POLL_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(300);
pub const LAUNCH_SETTLE: Duration = Duration::from_secs(5);
/// Checking ends at the latest this long after it began: the restore probe's 5 s cap plus margin.
/// The driver then concludes "document unavailable" (plan review I5, lead ruling 2026-10-07).
pub const CHECKING_WATCHDOG: Duration = Duration::from_secs(15);

/// The server's `ttl_seconds`, bounded; absent or 0 takes the stage's built-in value.
pub fn ttl(server: Option<u64>, stage: Stage) -> Duration {
    todo!("Task 6 step 3")
}

/// The server's `purchase.checkout.poll_seconds`, bounded; absent or 0 takes 3 s.
pub fn poll(server: Option<u64>) -> Duration {
    todo!("Task 6 step 3")
}

/// How a fetch concluded, as far as the schedule cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A usable document.
    Document,
    /// Any other HTTP answer: 401, 4xx, 5xx, an unreadable body, an unknown schema. The network works.
    Answered,
    /// 429, with its `Retry-After` when readable.
    RateLimited(Option<Duration>),
    /// DNS, connect, TLS, timeout, or the exchange broke before any status.
    Network,
}

#[derive(Debug, Clone, Default)]
pub struct Schedule {
    last_attempt: Option<Instant>,
    failures_since_document: u32,
    first_network_failure: Option<Instant>,
    offline: bool,
    rate_limited_until: Option<Instant>,
    plan_poll_until: Option<Instant>,
}

impl Schedule {
    pub fn offline(&self) -> bool {
        self.offline
    }

    pub fn last_attempt(&self) -> Option<Instant> {
        self.last_attempt
    }

    /// Record a concluded fetch. `ttl` is the 429 fallback. Returns `true` when Offline changed.
    pub fn record(&mut self, outcome: Outcome, now: Instant, ttl: Duration) -> bool {
        todo!("Task 6 step 3")
    }

    /// "Choose a plan on beebeeb.io" was clicked: poll every `poll` for 15 minutes from now.
    pub fn plan_opened(&mut self, now: Instant) {
        todo!("Task 6 step 3")
    }

    /// The plan poll runs (it pauses while offline and resumes inside its 15 minutes).
    pub fn plan_polling(&self, now: Instant) -> bool {
        todo!("Task 6 step 3")
    }

    pub fn next_fetch_at(&self, now: Instant, ttl: Duration, poll: Duration) -> Instant {
        todo!("Task 6 step 3")
    }

    /// A Beebeeb window got focus: fetch when the last fetch is older than `poll`.
    pub fn focus_wants_fetch(&self, now: Instant, poll: Duration) -> bool {
        todo!("Task 6 step 3")
    }
}

/// C-W4: the macOS launch decision waits at most 5 s after the restore probe completes or times out.
pub fn settle_deadline(probe_done: Instant) -> Instant {
    todo!("Task 6 step 3")
}

/// When Checking gives up and the driver concludes "document unavailable", counted from the moment
/// Checking began (a launch, or a session generation change).
pub fn checking_deadline(since: Instant) -> Instant {
    todo!("Task 6 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn ttl_and_poll_take_the_built_in_when_absent_or_zero_and_are_clamped() {
        assert_eq!(ttl(None, Stage::Account), s(60));
        assert_eq!(ttl(Some(0), Stage::Account), s(60));
        assert_eq!(ttl(None, Stage::PreAccount), s(300));
        assert_eq!(ttl(Some(5), Stage::Account), s(30));
        assert_eq!(ttl(Some(120), Stage::PreAccount), s(120));
        assert_eq!(ttl(Some(86_400), Stage::Account), s(900));
        assert_eq!(poll(None), s(3));
        assert_eq!(poll(Some(0)), s(3));
        assert_eq!(poll(Some(1)), s(2));
        assert_eq!(poll(Some(10)), s(10));
        assert_eq!(poll(Some(600)), s(30));
    }

    #[test]
    fn the_first_fetch_is_due_at_once_and_then_every_ttl() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        assert_eq!(schedule.next_fetch_at(t0, s(60), s(3)), t0);
        schedule.record(Outcome::Document, t0, s(60));
        assert_eq!(schedule.next_fetch_at(t0, s(60), s(3)), t0 + s(60));
    }

    #[test]
    fn every_3s_for_15_minutes_after_the_click_then_every_ttl() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Document, t0, s(60));
        schedule.plan_opened(t0);
        let mut now = t0;
        let mut fetches = 0;
        while schedule.plan_polling(now) {
            now = schedule.next_fetch_at(now, s(60), s(3));
            schedule.record(Outcome::Document, now, s(60));
            fetches += 1;
        }
        assert_eq!(fetches, 300, "15 minutes at 3 s");
        assert_eq!(now, t0 + s(900));
        assert_eq!(schedule.next_fetch_at(now, s(60), s(3)), now + s(60));
    }

    #[test]
    fn a_first_failure_retries_after_10s_and_a_second_returns_to_the_ttl() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Document, t0, s(60));
        schedule.record(Outcome::Answered, t0 + s(60), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(60), s(60), s(3)), t0 + s(70));
        schedule.record(Outcome::Answered, t0 + s(70), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(70), s(60), s(3)), t0 + s(130));
    }

    #[test]
    fn offline_needs_two_network_failures_at_least_10s_apart() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        assert!(!schedule.record(Outcome::Network, t0, s(60)));
        assert!(!schedule.offline(), "never after 1");
        assert!(!schedule.record(Outcome::Network, t0 + s(4), s(60)));
        assert!(!schedule.offline(), "not when the second came less than 10 s after the first");
        assert!(schedule.record(Outcome::Network, t0 + s(10), s(60)));
        assert!(schedule.offline(), "two failures at least 10 s apart");
    }

    #[test]
    fn an_http_answer_never_makes_offline() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        for n in 0..10 {
            schedule.record(Outcome::Answered, t0 + s(n * 15), s(60));
        }
        assert!(!schedule.offline());
    }

    /// Plan "Spec issues" 6.
    #[test]
    fn an_http_answer_clears_offline_and_only_a_2xx_resets_the_retry() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Network, t0, s(60));
        schedule.record(Outcome::Network, t0 + s(10), s(60));
        assert!(schedule.offline());
        assert!(schedule.record(Outcome::Answered, t0 + s(40), s(60)), "the overlay changed");
        assert!(!schedule.offline());
        assert_eq!(schedule.next_fetch_at(t0 + s(40), s(60), s(3)), t0 + s(100), "three failures since the last document: back to the ttl");
        schedule.record(Outcome::Network, t0 + s(100), s(60));
        assert!(!schedule.offline(), "the network-failure count started again");
    }

    #[test]
    fn one_document_clears_offline() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Network, t0, s(60));
        schedule.record(Outcome::Network, t0 + s(10), s(60));
        assert!(schedule.record(Outcome::Document, t0 + s(40), s(60)));
        assert!(!schedule.offline());
        assert_eq!(schedule.next_fetch_at(t0 + s(40), s(60), s(3)), t0 + s(100));
    }

    #[test]
    fn offline_reprobes_every_30s() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Network, t0, s(60));
        schedule.record(Outcome::Network, t0 + s(10), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(10), s(60), s(3)), t0 + s(40));
        schedule.record(Outcome::Network, t0 + s(40), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(40), s(60), s(3)), t0 + s(70));
    }

    #[test]
    fn a_rate_limit_waits_for_retry_after_or_the_ttl_capped_at_300s() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::RateLimited(Some(s(20))), t0, s(60));
        assert_eq!(schedule.next_fetch_at(t0, s(60), s(3)), t0 + s(20));
        schedule.record(Outcome::RateLimited(None), t0 + s(20), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(20), s(60), s(3)), t0 + s(80), "no Retry-After: the ttl");
        schedule.record(Outcome::RateLimited(Some(s(3600))), t0 + s(80), s(60));
        assert_eq!(schedule.next_fetch_at(t0 + s(80), s(60), s(3)), t0 + s(380), "capped at 300 s");
        schedule.plan_opened(t0 + s(80));
        assert_eq!(schedule.next_fetch_at(t0 + s(81), s(60), s(3)), t0 + s(380), "the plan poll waits for the rate limit too");
    }

    #[test]
    fn a_network_failure_after_a_rate_limit_never_retries_at_once() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::RateLimited(Some(s(20))), t0, s(60));
        schedule.record(Outcome::Network, t0 + s(20), s(60));
        assert!(schedule.next_fetch_at(t0 + s(20), s(60), s(3)) > t0 + s(20));
    }

    #[test]
    fn focus_fetches_only_when_the_last_fetch_is_older_than_poll() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.focus_wants_fetch(t0, s(3)), "nothing fetched yet");
        schedule.record(Outcome::Document, t0, s(60));
        assert!(!schedule.focus_wants_fetch(t0 + s(2), s(3)));
        assert!(schedule.focus_wants_fetch(t0 + s(4), s(3)));
    }

    #[test]
    fn the_plan_poll_pauses_offline_and_resumes_within_its_15_minutes() {
        let t0 = Instant::now();
        let mut schedule = Schedule::default();
        schedule.record(Outcome::Document, t0, s(60));
        schedule.plan_opened(t0);
        schedule.record(Outcome::Network, t0 + s(3), s(60));
        schedule.record(Outcome::Network, t0 + s(13), s(60));
        assert!(schedule.offline());
        assert!(!schedule.plan_polling(t0 + s(13)), "paused while offline");
        assert_eq!(schedule.next_fetch_at(t0 + s(13), s(60), s(3)), t0 + s(43), "the 30 s probe replaces it");
        schedule.record(Outcome::Document, t0 + s(43), s(60));
        assert!(schedule.plan_polling(t0 + s(43)), "back online inside the 15 minutes");
        assert_eq!(schedule.next_fetch_at(t0 + s(43), s(60), s(3)), t0 + s(46));
        assert!(!schedule.plan_polling(t0 + s(900)), "the 15 minutes do not restart");
    }

    #[test]
    fn the_launch_settle_ends_5s_after_the_probe() {
        let t0 = Instant::now();
        assert_eq!(settle_deadline(t0), t0 + s(5));
    }

    #[test]
    fn checking_gives_up_15s_after_it_began() {
        let t0 = Instant::now();
        assert_eq!(checking_deadline(t0), t0 + s(15), "the 5 s probe cap plus margin");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Add `pub mod policy;`. Run with filter `account_view::policy`.
Expected: `test result: FAILED. 0 passed; 15 failed`.

- [ ] **Step 3: Implement**

```rust
pub fn ttl(server: Option<u64>, stage: Stage) -> Duration {
    let built_in = match stage {
        Stage::Account => ACCOUNT_TTL,
        Stage::PreAccount => PRE_ACCOUNT_TTL,
    };
    bounded(server, built_in, TTL_MIN, TTL_MAX)
}

pub fn poll(server: Option<u64>) -> Duration {
    bounded(server, PLAN_POLL, POLL_MIN, POLL_MAX)
}

fn bounded(server: Option<u64>, built_in: Duration, min: Duration, max: Duration) -> Duration {
    match server {
        None | Some(0) => built_in,
        Some(seconds) => Duration::from_secs(seconds).clamp(min, max),
    }
}

impl Schedule {
    pub fn record(&mut self, outcome: Outcome, now: Instant, ttl: Duration) -> bool {
        let was_offline = self.offline;
        self.last_attempt = Some(now);
        match outcome {
            Outcome::Network => {
                self.failures_since_document += 1;
                self.rate_limited_until = None;
                match self.first_network_failure {
                    None => self.first_network_failure = Some(now),
                    Some(first) if now.duration_since(first) >= OFFLINE_MIN_GAP => self.offline = true,
                    Some(_) => {}
                }
            }
            reached => {
                self.first_network_failure = None;
                self.offline = false;
                match reached {
                    Outcome::Document => {
                        self.failures_since_document = 0;
                        self.rate_limited_until = None;
                    }
                    Outcome::RateLimited(retry_after) => {
                        self.failures_since_document += 1;
                        self.rate_limited_until = Some(now + retry_after.unwrap_or(ttl).min(RETRY_AFTER_CAP));
                    }
                    Outcome::Answered | Outcome::Network => {
                        self.failures_since_document += 1;
                        self.rate_limited_until = None;
                    }
                }
            }
        }
        was_offline != self.offline
    }

    pub fn plan_opened(&mut self, now: Instant) {
        self.plan_poll_until = Some(now + PLAN_POLL_WINDOW);
    }

    pub fn plan_polling(&self, now: Instant) -> bool {
        !self.offline && self.plan_poll_until.is_some_and(|until| now < until)
    }

    pub fn next_fetch_at(&self, now: Instant, ttl: Duration, poll: Duration) -> Instant {
        let Some(last) = self.last_attempt else {
            return now;
        };
        if self.offline {
            return last + OFFLINE_PROBE;
        }
        if let Some(until) = self.rate_limited_until {
            return until;
        }
        if self.plan_polling(now) {
            return last + poll;
        }
        if self.failures_since_document == 1 {
            return last + FIRST_FAILURE_RETRY;
        }
        last + ttl
    }

    pub fn focus_wants_fetch(&self, now: Instant, poll: Duration) -> bool {
        self.last_attempt.is_none_or(|last| now.duration_since(last) > poll)
    }
}

pub fn settle_deadline(probe_done: Instant) -> Instant {
    probe_done + LAUNCH_SETTLE
}

pub fn checking_deadline(since: Instant) -> Instant {
    since + CHECKING_WATCHDOG
}
```

(Remove the step-1 `todo!()` bodies of the same methods; the struct, `offline()` and `last_attempt()` stay.)

- [ ] **Step 4: Run the tests to see them pass**

Expected: `test result: ok. 15 passed; 0 failed`.

- [ ] **Step 5: Mutation-check**

1. Change `>= OFFLINE_MIN_GAP` to `>= Duration::ZERO`. Expected: `offline_needs_two_network_failures_at_least_10s_apart`, at the un-messaged `assertion failed: !schedule.record(Outcome::Network, t0 + s(4), s(60))` (the second failure already flips Offline, so the assert before the "not when the second came…" message fails first). Revert.
2. Change `!self.offline && self.plan_poll_until…` to `self.plan_poll_until…`. Expected: `the_plan_poll_pauses_offline_and_resumes_within_its_15_minutes`. Revert.
3. Remove `self.rate_limited_until = None;` from the `Network` arm. Expected: `a_network_failure_after_a_rate_limit_never_retries_at_once`. Revert.
4. In `bounded`, replace `.clamp(min, max)` with `.max(min)`. Expected: `ttl_and_poll_take_the_built_in_when_absent_or_zero_and_are_clamped` at `ttl(Some(86_400), Stage::Account)` (left `86400s`, right `900s`). Revert.
5. In `poll`, replace `PLAN_POLL` with `ACCOUNT_TTL`. Expected: the same test at `poll(None)` (left `60s`, right `3s`). Revert.
6. In `checking_deadline`, replace `CHECKING_WATCHDOG` with `LAUNCH_SETTLE`. Expected: `checking_gives_up_15s_after_it_began` (left `t0 + 5s`). Revert.

- [ ] **Step 6: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t6-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/policy.rs src-tauri/src/account_view/mod.rs
git commit -m "desktop: schedule onboarding document fetches and the offline overlay on an injected clock" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/policy.rs src-tauri/src/account_view/mod.rs
git show --stat HEAD
```

---

## Task 7: The cached account part in `state.db`, its trace and its purges (spec C-D5)

**Lane R.** Touches spec A's R10 code: the account binding's classification of tables, the purges, and R8's trace list.

**Files:**
- Modify: `src-tauri/src/state_db.rs` (constants near `ACCOUNT_TABLES`; `StateDb::open`'s base schema; new methods next to `owner`/`set_owner`; `purge_all_local_state`; `clear_account_data`; `finish_windows_signout`; the classification test)
- Modify: `src-tauri/src/reauth.rs` (`LocalTraces`)
- Modify: `src-tauri/src/lib.rs` (`gather_local_facts`, a new helper beside `queued_or_staged_present`, one test in the R8 test module that defines `Fixture`)

**Interfaces:**
- Consumes: spec A's `StateDb`, `account_binding::Identity`, `reauth::LocalTraces`, `gather_local_facts`, `LocalSources`.
- Produces:
  - `state_db::OnboardingCacheRow { identity: Identity, fetched_at: i64, account_part: String }`
  - `StateDb::onboarding_cache(&self) -> Result<Option<OnboardingCacheRow>>`, `StateDb::set_onboarding_cache(&self, identity: &Identity, fetched_at: i64, account_part: &str) -> Result<()>`, `StateDb::delete_onboarding_cache(&self) -> Result<()>`
  - `reauth::LocalTraces.onboarding_cache: bool` (counted by `any()`)
  - `lib.rs`: `fn onboarding_cache_present(sources: &LocalSources) -> bool` (not Windows)

- [ ] **Step 1: Write the failing tests**

In `state_db.rs`'s `mod tests` (it already has `use tempfile::tempdir;`):

```rust
    #[test]
    fn the_onboarding_cache_row_round_trips_and_a_second_write_replaces_it() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.onboarding_cache().unwrap(), None);
        let sam = crate::account_binding::Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
        db.set_onboarding_cache(&sam, 1_791_000_000, r#"{"blocking":true}"#).unwrap();
        assert_eq!(
            db.onboarding_cache().unwrap(),
            Some(OnboardingCacheRow { identity: sam.clone(), fetched_at: 1_791_000_000, account_part: r#"{"blocking":true}"#.into() })
        );
        db.set_onboarding_cache(&sam, 1_791_000_060, r#"{"blocking":false}"#).unwrap();
        let row = db.onboarding_cache().unwrap().unwrap();
        assert_eq!((row.fetched_at, row.account_part.as_str()), (1_791_000_060, r#"{"blocking":false}"#));
        db.delete_onboarding_cache().unwrap();
        assert_eq!(db.onboarding_cache().unwrap(), None);
    }

    /// Plan "Spec issues" 9: a sign-in writes the row before the first engine start; R10 must not see it as data.
    #[test]
    fn the_cache_row_is_never_account_data() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_onboarding_cache(&crate::account_binding::Identity::new(Some("u-1"), None), 1, "{}").unwrap();
        assert!(!db.has_account_data().unwrap());
        assert_eq!(db.owner().unwrap(), None);
    }

    #[test]
    fn a_sign_out_purge_and_every_account_reset_delete_the_cache_row() {
        let sam = crate::account_binding::Identity::new(Some("u-1"), Some("sam@beebeeb.io"));
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_onboarding_cache(&sam, 1, "{}").unwrap();
        db.purge_all_local_state().unwrap();
        assert_eq!(db.onboarding_cache().unwrap(), None, "sign-out by choice and the switch purge it");
        for owe_finder_removal in [false, true] {
            db.set_onboarding_cache(&sam, 1, "{}").unwrap();
            db.clear_account_data(owe_finder_removal).unwrap();
            assert_eq!(db.onboarding_cache().unwrap(), None, "R10's reset (owe_finder_removal = {owe_finder_removal})");
        }
    }

    #[test]
    fn the_windows_sign_out_deletes_the_cache_row() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_onboarding_cache(&crate::account_binding::Identity::new(Some("u-1"), None), 1, "{}").unwrap();
        db.finish_windows_signout().unwrap();
        assert_eq!(db.onboarding_cache().unwrap(), None);
    }
```

In `reauth.rs`'s tests:

```rust
    #[test]
    fn a_cached_onboarding_part_is_a_retained_trace() {
        assert!(!LocalTraces::default().any());
        assert!(LocalTraces { onboarding_cache: true, ..LocalTraces::default() }.any());
    }
```

In `lib.rs`, in the R8 test module that defines `Fixture` (the one holding `a_poisoned_lock_counts_as_a_trace_in_each_of_its_three_places`):

```rust
        /// Spec 2026-10-07 C-D5: the cached onboarding part is a trace, so another account after it is a switch.
        #[test]
        fn a_cached_onboarding_part_is_counted_when_someone_signs_in() {
            let fx = Fixture::new();
            assert!(!gather_local_facts(&fx.state, &fx.acct, &fx.sources()).traces.onboarding_cache, "a clean world holds nothing");
            fx.db().set_onboarding_cache(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")), 1, "{}").unwrap();
            let traces = gather_local_facts(&fx.state, &fx.acct, &fx.sources()).traces;
            assert!(traces.onboarding_cache && traces.any());
            assert_ne!(fx.classify(&kim()).0, reauth::SignInKind::Fresh, "another account is never a first sign-in");
        }
```

- [ ] **Step 2: Run them to see them fail**

Run: the loop command with the six new names as the filter (`-- the_onboarding_cache_row_round_trips_and_a_second_write_replaces_it the_cache_row_is_never_account_data a_sign_out_purge_and_every_account_reset_delete_the_cache_row the_windows_sign_out_deletes_the_cache_row a_cached_onboarding_part_is_a_retained_trace a_cached_onboarding_part_is_counted_when_someone_signs_in`; the substring `onboarding_cache` matches only the first). Expected: compile errors `no method named set_onboarding_cache` / `no field onboarding_cache`. Paste them; a compile error is this step's red.

- [ ] **Step 3: Implement in `state_db.rs`**

After `DEVICE_TABLES`:

```rust
/// Spec 2026-10-07 C-D5: bound to the account they were fetched for, holding no user content and no account data. A
/// sign-in writes them before the first engine start, so counting them as account data would change R10's binding
/// (plan "Spec issues" 9). Emptied by every reset (`clear_account_data`) and every purge.
const ACCOUNT_BOUND_TABLES: [&str; 1] = ["onboarding_account_cache"];
```

In `StateDb::open`, inside the base-schema `execute_batch` string, directly after the `upload_resume` table:

```sql
            CREATE TABLE IF NOT EXISTS onboarding_account_cache (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                owner_user_id TEXT,
                owner_email TEXT,
                fetched_at INTEGER NOT NULL,
                account_part TEXT NOT NULL
            );
```

After `set_owner`:

```rust
    /// Spec 2026-10-07 C-D5: the cached account part of the onboarding document, the time it was fetched (Unix
    /// seconds) and the account it was fetched for. One row at most. `account_part` is JSON that `account_view` owns.
    pub fn onboarding_cache(&self) -> Result<Option<OnboardingCacheRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT owner_user_id, owner_email, fetched_at, account_part FROM onboarding_account_cache WHERE id = 1",
            [],
            |row| {
                Ok(OnboardingCacheRow {
                    identity: crate::account_binding::Identity::new(
                        row.get::<_, Option<String>>(0)?.as_deref(),
                        row.get::<_, Option<String>>(1)?.as_deref(),
                    ),
                    fetched_at: row.get(2)?,
                    account_part: row.get(3)?,
                })
            },
        )
        .optional()
    }

    pub fn set_onboarding_cache(&self, identity: &crate::account_binding::Identity, fetched_at: i64, account_part: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO onboarding_account_cache (id, owner_user_id, owner_email, fetched_at, account_part) VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET owner_user_id = excluded.owner_user_id, owner_email = excluded.owner_email, \
             fetched_at = excluded.fetched_at, account_part = excluded.account_part",
            params![identity.user_id, identity.email, fetched_at, account_part],
        )?;
        Ok(())
    }

    pub fn delete_onboarding_cache(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM onboarding_account_cache", [])?;
        Ok(())
    }
```

Next to the other public row structs (for example after `UploadFinalization`):

```rust
/// Spec 2026-10-07 C-D5: one cached onboarding account part (see `StateDb::onboarding_cache`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingCacheRow {
    pub identity: crate::account_binding::Identity,
    pub fetched_at: i64,
    pub account_part: String,
}
```

In `purge_all_local_state`, directly after `tx.execute("DELETE FROM transfer_activity", [])?;`:

```rust
        // Spec 2026-10-07 C-D5: the leaving account's cached onboarding part goes too. The table is named here
        // explicitly, like every other table this purge clears.
        tx.execute("DELETE FROM onboarding_account_cache", [])?;
```

In `clear_account_data`, change the loop head to:

```rust
        for table in ACCOUNT_TABLES.iter().chain(DEVICE_TABLES.iter()).chain(ACCOUNT_BOUND_TABLES.iter()) {
```

In `finish_windows_signout`, append `DELETE FROM onboarding_account_cache;` to the `execute_batch` string (after `DELETE FROM bandwidth_samples;`).

In `every_table_is_classified_for_the_account_binding`, accept the new list:

```rust
            assert!(
                ACCOUNT_TABLES.contains(&table.as_str())
                    || DEVICE_TABLES.contains(&table.as_str())
                    || ACCOUNT_BOUND_TABLES.contains(&table.as_str())
                    || table == "sync_state",
                "{table}: list it in ACCOUNT_TABLES (R10), DEVICE_TABLES (no account data) or ACCOUNT_BOUND_TABLES (bound to an account, no user content)"
            );
```

- [ ] **Step 4: Implement the trace**

`reauth.rs`, in `LocalTraces` after `queued_or_staged`:

```rust
    /// Spec 2026-10-07 C-D5: the cached onboarding account part (`state.db`).
    pub onboarding_cache: bool,
```

and in `any()` add `|| self.onboarding_cache` after `|| self.queued_or_staged`.

Find every struct literal that lists all of `LocalTraces`'s fields: `git -C $WT grep -n 'LocalTraces {' -- src-tauri/src`. Literals ending in `..LocalTraces::default()` need nothing. In `gather_local_facts` (`lib.rs`) add the field:

```rust
        onboarding_cache: onboarding_cache_present(sources),
```

and beside `queued_or_staged_present`:

```rust
/// Spec 2026-10-07 C-D5: is a cached onboarding account part stored? An unreadable `state.db` counts as present
/// (fail closed), like every other trace.
#[cfg(not(target_os = "windows"))]
fn onboarding_cache_present(sources: &LocalSources) -> bool {
    match sources.state_db() {
        Ok(Some(db)) => db.onboarding_cache().map(|row| row.is_some()).unwrap_or(true),
        Ok(None) => false,
        Err(_) => true,
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run with the six names (`-- the_onboarding_cache_row_round_trips_and_a_second_write_replaces_it the_cache_row_is_never_account_data a_sign_out_purge_and_every_account_reset_delete_the_cache_row the_windows_sign_out_deletes_the_cache_row a_cached_onboarding_part_is_a_retained_trace a_cached_onboarding_part_is_counted_when_someone_signs_in`), then with filter `state_db::tests::every_table_is_classified`, then the whole `reauth` and R8 modules (filters `reauth::` and the R8 module's name).
Expected: `test result: ok. 6 passed; 0 failed` for the six (on macOS and Linux; on Windows the R8 test is compiled out and the line says 5); `every_table_is_classified_for_the_account_binding` passes; the `reauth::` and R8 counts are Task 0's baseline counts plus 1 each.

- [ ] **Step 6: Mutation-check**

1. Remove the `DELETE FROM onboarding_account_cache` line from `purge_all_local_state`. Expected: `a_sign_out_purge_and_every_account_reset_delete_the_cache_row` ("sign-out by choice and the switch purge it"). Revert.
2. Add `"onboarding_account_cache"` to `ACCOUNT_TABLES` (and its length) and remove it from `ACCOUNT_BOUND_TABLES`'s loop. Expected: `the_cache_row_is_never_account_data`. Revert.
3. Replace `onboarding_cache_present(sources)` with `false`. Expected: `a_cached_onboarding_part_is_counted_when_someone_signs_in`. Revert.

- [ ] **Step 7: Run the whole lib under the scratch-HOME recipe and commit**

```bash
cd $WT/src-tauri && SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked --lib > $EVID/t7-lib.log 2>&1; echo "rc=$?"
rm -rf "$SCRATCH"; /usr/bin/grep "test result:" $EVID/t7-lib.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t7-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/state_db.rs src-tauri/src/reauth.rs src-tauri/src/lib.rs
git commit -m "desktop: keep one cached onboarding account part per account in state.db" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/state_db.rs src-tauri/src/reauth.rs src-tauri/src/lib.rs
git show --stat HEAD
```

Expected: `test result: ok. N passed; 0 failed` with N = `B_lib` + 72 (the running total in Global Constraints).

---

## Task 8: The fetch — request, headers and outcome (`account_view::fetch`, spec C-D1, C-D2, §10)

**Lane R.** Real HTTP against a loopback one-shot server; no production endpoint is ever contacted.

**Files:**
- Create: `src-tauri/src/account_view/fetch.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod fetch;`)

**Interfaces:**
- Consumes: `doc::{self, Doc, Unavailable}` (Task 2), `crate::api_client::provenance_headers()` and `crate::link_health::classify_reqwest` (spec A, unchanged), `crate::surfaces::policy::Platform`.
- Produces (`crate::account_view::fetch`):
  - `SCHEMA_HEADER = "X-Beebeeb-Onboarding-Schema"`, `OS_HEADER = "X-Beebeeb-Client-OS"`, `TIMEOUT` (10 s)
  - `fn os_header_value(Platform) -> &'static str` (`macos`, `windows`, `linux`), `fn request_headers(Platform) -> HeaderMap`
  - `fn client(Platform) -> Result<reqwest::Client, String>`, `fn client_with_timeout(Platform, Duration) -> Result<reqwest::Client, String>`
  - `enum UnavailableKind { Http(u16), Unreadable, UnknownSchema }`
  - `enum FetchOutcome { Document(Doc), Unauthorized, RateLimited { retry_after: Option<Duration> }, Unavailable(UnavailableKind), Network }` (PartialEq; its `Debug` never prints the document)
  - `async fn fetch(client: &reqwest::Client, base_url: &str, bearer: Option<&str>) -> FetchOutcome`
  - `fn retry_after(&HeaderMap) -> Option<Duration>`

- [ ] **Step 1: Write `fetch.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 C-D1, C-D2, §10: one `GET /api/v1/onboarding` and what came of it. Anonymous while
//! signed out (stage `pre_account`), with the session's Bearer while signed in (stage `account`); the
//! server answers a bad Bearer with 401 and never falls back to anonymous. The body leaves this module
//! only as a parsed `Doc` (C-D6): it is never logged, and `FetchOutcome`'s `Debug` prints no part of it.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

use super::doc::{self, Doc, Unavailable};
use crate::surfaces::policy::Platform;

pub const SCHEMA_HEADER: &str = "X-Beebeeb-Onboarding-Schema";
pub const OS_HEADER: &str = "X-Beebeeb-Client-OS";
pub const TIMEOUT: Duration = Duration::from_secs(10);

pub fn os_header_value(platform: Platform) -> &'static str {
    todo!("Task 8 step 3")
}

/// Today's provenance headers (`X-Beebeeb-Client`, `X-Beebeeb-Client-Version`) plus the two this request adds.
/// Without the OS header the server gives the desktop purchase surface `none` and omits `poll_seconds`.
pub fn request_headers(platform: Platform) -> HeaderMap {
    todo!("Task 8 step 3")
}

pub fn client(platform: Platform) -> Result<reqwest::Client, String> {
    client_with_timeout(platform, TIMEOUT)
}

pub fn client_with_timeout(platform: Platform, timeout: Duration) -> Result<reqwest::Client, String> {
    todo!("Task 8 step 3")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableKind {
    /// Any HTTP status that is not a usable document: 5xx, a 400 for the schema header, other 4xx.
    Http(u16),
    Unreadable,
    UnknownSchema,
}

#[derive(Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Document(Doc),
    Unauthorized,
    RateLimited { retry_after: Option<Duration> },
    Unavailable(UnavailableKind),
    /// DNS, connect, TLS, timeout, or the exchange broke before any HTTP status.
    Network,
}

/// C-D6: never the document's content, only which kind of outcome it was.
impl std::fmt::Debug for FetchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Document(doc) => write!(f, "Document({:?})", doc.stage()),
            Self::Unauthorized => f.write_str("Unauthorized"),
            Self::RateLimited { retry_after } => write!(f, "RateLimited({retry_after:?})"),
            Self::Unavailable(kind) => write!(f, "Unavailable({kind:?})"),
            Self::Network => f.write_str("Network"),
        }
    }
}

pub async fn fetch(client: &reqwest::Client, base_url: &str, bearer: Option<&str>) -> FetchOutcome {
    todo!("Task 8 step 3")
}

/// `Retry-After` in seconds. The HTTP-date form, or anything unreadable, is `None` (the ttl then decides).
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    todo!("Task 8 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::doc::tests::fixture;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    struct OneShot {
        base_url: String,
        request: mpsc::Receiver<String>,
    }

    /// Answers exactly one request with `response` (a whole HTTP/1.1 response) and hands back the request
    /// head, lower-cased (header names are case-insensitive).
    fn serve_once(response: String) -> OneShot {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, request) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut buffer = [0u8; 4096];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                head.extend_from_slice(&buffer[..read]);
            }
            let _ = tx.send(String::from_utf8_lossy(&head).to_lowercase());
            let _ = stream.write_all(response.as_bytes());
        });
        OneShot { base_url, request }
    }

    fn response(status: &str, extra_headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn body(name: &str) -> String {
        String::from_utf8(fixture(name)).unwrap()
    }

    async fn fetch_once(response_text: String, bearer: Option<&str>) -> (FetchOutcome, String) {
        let server = serve_once(response_text);
        let outcome = fetch(&client(Platform::Macos).unwrap(), &server.base_url, bearer).await;
        let head = server.request.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or_default();
        (outcome, head)
    }

    #[tokio::test]
    async fn the_request_carries_both_onboarding_headers_and_the_provenance_headers() {
        for (platform, os) in [(Platform::Macos, "macos"), (Platform::Windows, "windows"), (Platform::Linux, "linux")] {
            let server = serve_once(response("200 OK", "", &body("pre_account.desktop.json")));
            let outcome = fetch(&client(platform).unwrap(), &server.base_url, None).await;
            assert!(matches!(outcome, FetchOutcome::Document(_)), "{os}: {outcome:?}");
            let head = server.request.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            assert!(head.starts_with("get /api/v1/onboarding http/1.1\r\n"), "{os}: {head}");
            assert!(head.contains("\r\nx-beebeeb-onboarding-schema: 1\r\n"), "{os}: {head}");
            assert!(head.contains(&format!("\r\nx-beebeeb-client-os: {os}\r\n")), "{os}: {head}");
            assert!(head.contains("\r\nx-beebeeb-client: desktop\r\n"), "{os}: {head}");
            assert!(head.contains("\r\nx-beebeeb-client-version: "), "{os}: {head}");
        }
    }

    #[tokio::test]
    async fn signed_out_is_anonymous_and_signed_in_sends_the_bearer() {
        let (_, head) = fetch_once(response("200 OK", "", &body("pre_account.desktop.json")), None).await;
        assert!(!head.contains("\r\nauthorization:"), "{head}");
        let (_, head) = fetch_once(response("200 OK", "", &body("account.active.web.json")), Some("tok-1")).await;
        assert!(head.contains("\r\nauthorization: bearer tok-1\r\n"), "{head}");
    }

    #[tokio::test]
    async fn a_2xx_document_is_parsed() {
        let (outcome, _) = fetch_once(response("200 OK", "", &body("account.active.web.json")), Some("tok")).await;
        assert_eq!(outcome, FetchOutcome::Document(doc::parse(&fixture("account.active.web.json")).unwrap()));
    }

    #[tokio::test]
    async fn a_401_is_unauthorized() {
        let (outcome, _) = fetch_once(response("401 Unauthorized", "", r#"{"error":"unauthorized"}"#), Some("tok")).await;
        assert_eq!(outcome, FetchOutcome::Unauthorized);
    }

    #[tokio::test]
    async fn a_429_reads_a_seconds_retry_after_and_nothing_else() {
        let (outcome, _) = fetch_once(response("429 Too Many Requests", "retry-after: 17\r\n", ""), None).await;
        assert_eq!(outcome, FetchOutcome::RateLimited { retry_after: Some(Duration::from_secs(17)) });
        let (outcome, _) = fetch_once(response("429 Too Many Requests", "retry-after: Wed, 21 Oct 2026 07:28:00 GMT\r\n", ""), None).await;
        assert_eq!(outcome, FetchOutcome::RateLimited { retry_after: None });
        let (outcome, _) = fetch_once(response("429 Too Many Requests", "", ""), None).await;
        assert_eq!(outcome, FetchOutcome::RateLimited { retry_after: None });
    }

    #[tokio::test]
    async fn any_other_status_is_unavailable_with_its_code_and_never_network() {
        for (status, code) in [("500 Internal Server Error", 500), ("503 Service Unavailable", 503), ("400 Bad Request", 400), ("404 Not Found", 404)] {
            let (outcome, _) = fetch_once(response(status, "", "{}"), None).await;
            assert_eq!(outcome, FetchOutcome::Unavailable(UnavailableKind::Http(code)), "{status}");
        }
    }

    #[tokio::test]
    async fn an_unreadable_body_and_an_unknown_schema_are_unavailable() {
        let (outcome, _) = fetch_once(response("200 OK", "", "<html>maintenance</html>"), None).await;
        assert_eq!(outcome, FetchOutcome::Unavailable(UnavailableKind::Unreadable));
        let v2 = body("pre_account.desktop.json").replacen("\"schema\": 1", "\"schema\": 2", 1);
        assert!(v2.contains("\"schema\": 2"), "the fixture's spelling of the schema field changed");
        let (outcome, _) = fetch_once(response("200 OK", "", &v2), None).await;
        assert_eq!(outcome, FetchOutcome::Unavailable(UnavailableKind::UnknownSchema));
    }

    #[tokio::test]
    async fn a_body_that_breaks_after_the_status_is_unreadable_not_network() {
        let broken = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 500\r\nconnection: close\r\n\r\n{\"schema\":1,".to_string();
        let (outcome, _) = fetch_once(broken, None).await;
        assert_eq!(outcome, FetchOutcome::Unavailable(UnavailableKind::Unreadable));
    }

    #[tokio::test]
    async fn a_refused_connection_is_a_network_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(fetch(&client(Platform::Macos).unwrap(), &base_url, None).await, FetchOutcome::Network);
    }

    #[tokio::test]
    async fn a_timeout_is_a_network_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(3));
            drop(stream);
        });
        let client = client_with_timeout(Platform::Macos, Duration::from_millis(300)).unwrap();
        assert_eq!(fetch(&client, &base_url, None).await, FetchOutcome::Network);
    }

    #[test]
    fn the_debug_output_never_holds_the_document() {
        let doc = doc::parse(&fixture("account.trial_ended.ios.json")).unwrap();
        let line = doc.account_part().unwrap().copy["trial_ended_over_allowance"].clone();
        let printed = format!("{:?}", FetchOutcome::Document(doc));
        assert_eq!(printed, "Document(Account)");
        assert!(!printed.contains(&line));
    }
}
```

The `v2` replacement assumes the fixture spells `"schema": 1` with one space (as the vendored files do); the assertion right after it fails loudly if that ever changes.

- [ ] **Step 2: Run the tests to see them fail**

Add `pub mod fetch;`. Run with filter `account_view::fetch`.
Expected: `test result: FAILED. 1 passed; 10 failed` (`the_debug_output_never_holds_the_document` passes already: its `Debug` is written in step 1; the other ten hit `todo!()`). Record that the Debug test is mutation-checked in step 5 instead.

- [ ] **Step 3: Implement**

```rust
pub fn os_header_value(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Windows => "windows",
        Platform::Linux => "linux",
    }
}

pub fn request_headers(platform: Platform) -> HeaderMap {
    let mut headers = crate::api_client::provenance_headers();
    headers.insert(SCHEMA_HEADER, HeaderValue::from_static("1"));
    headers.insert(OS_HEADER, HeaderValue::from_static(os_header_value(platform)));
    headers
}

pub fn client_with_timeout(platform: Platform, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .default_headers(request_headers(platform))
        .build()
        .map_err(|e| format!("onboarding client: {e}"))
}

pub async fn fetch(client: &reqwest::Client, base_url: &str, bearer: Option<&str>) -> FetchOutcome {
    let mut request = client.get(format!("{base_url}/api/v1/onboarding"));
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        // `classify_reqwest` minus its HTTP-status arm (§10): a send error never carries a status, and an error it
        // cannot place as a connectivity failure is not one.
        Err(error) if error.status().is_none() && crate::link_health::classify_reqwest(&error).is_some() => {
            return FetchOutcome::Network;
        }
        Err(_) => return FetchOutcome::Unavailable(UnavailableKind::Unreadable),
    };
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return FetchOutcome::Unauthorized;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return FetchOutcome::RateLimited { retry_after: retry_after(response.headers()) };
    }
    if !status.is_success() {
        return FetchOutcome::Unavailable(UnavailableKind::Http(status.as_u16()));
    }
    match response.bytes().await {
        Ok(body) => match doc::parse(&body) {
            Ok(doc) => FetchOutcome::Document(doc),
            Err(Unavailable::UnknownSchema) => FetchOutcome::Unavailable(UnavailableKind::UnknownSchema),
            Err(Unavailable::Unreadable) => FetchOutcome::Unavailable(UnavailableKind::Unreadable),
        },
        // The status arrived: whatever broke afterwards, the server answered.
        Err(_) => FetchOutcome::Unavailable(UnavailableKind::Unreadable),
    }
}

pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers.get(RETRY_AFTER)?.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}
```

- [ ] **Step 4: Run the tests to see them pass**

Expected: `test result: ok. 11 passed; 0 failed`.

- [ ] **Step 5: Mutation-check**

1. Replace `os_header_value`'s `Platform::Linux => "linux"` with `Platform::Linux => "macos"`. Expected: `the_request_carries_both_onboarding_headers_and_the_provenance_headers` fails for `linux`. Revert.
2. Delete the `headers.insert(SCHEMA_HEADER, …)` line. Expected: the same test fails on `x-beebeeb-onboarding-schema`. Revert.
3. Change the first `Err(error) if …` arm to return `FetchOutcome::Unavailable(UnavailableKind::Unreadable)`. Expected: `a_refused_connection_is_a_network_failure` and `a_timeout_is_a_network_failure`. Revert.
4. In `Debug`, print `Document({doc:?})`. Expected: `the_debug_output_never_holds_the_document`. Revert.

- [ ] **Step 6: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t8-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/fetch.rs src-tauri/src/account_view/mod.rs
git commit -m "desktop: fetch the onboarding document with the schema and OS headers" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/fetch.rs src-tauri/src/account_view/mod.rs
git show --stat HEAD
```

---

## Task 9: The account gate at the point of action (spec C-R10)

**Lane R.** Changes spec A's engine-start gate and its Finder reconciler. Every engine start reads the gate under the engine slot; the reconciler's add reads it immediately before it acts; both read it against the session generation that is current at that moment, and a generation the driver has not derived for is closed (Checking; plan review I1, lead ruling 2026-10-07). A closed gate is "held", never a failure; blocking never removes Finder.

**Before you start:** read `$EVID/t0-record.md`. This task uses three of its facts: the session generation's read function (written `crate::session_generation()` below; use the recorded name), whether `SESSION_TRANSITION` is Windows-only, and the six `spawn_bound_engine` callers.

**Files:**
- Create: `src-tauri/src/account_view/gate.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod gate;`)
- Modify: `src-tauri/src/lib.rs` (`AppState` + its `Default`; `StartPermit`, `EngineStart` and `StartPermit`'s `Debug`; `authorize_engine_start`; `start_engine_bound`; the six callers of `spawn_bound_engine`; new `hold_for_account`; tests)
- Modify: `src-tauri/src/finder_setup/error.rs` (`app_code::ACCOUNT_BLOCKED`)
- Modify: `src-tauri/src/finder_setup/core.rs` (`Trigger::AccountHold`, `Trigger::AccountReady`, `hold`, the blocked-result rule)
- Modify: `src-tauri/src/finder_setup/driver.rs` (`Event::AccountHold`, `FinderSetupHandle::account_hold`)
- Modify: `src-tauri/src/finder_setup/macos_ports.rs` (`check_epoch`, the add's guard)

**Interfaces:**
- Consumes: spec A's `authorize_engine_start`, `start_engine_bound`, `spawn_bound_engine` and its six callers, `stop_engine_in_slot`, `finder_setup::{core, driver, macos_ports, error}`; the test fixtures `AuthorizeFixture::with_session`, `local_data_of`, `alice()`, `bob()`, `LocalDataPaths::for_test`, and the source helpers `finder_setup_command_tests::{body_between, production_source}`.
- Produces:
  - `crate::account_view::gate::{AccountGate, GateValue}`: `Default` (unarmed: holds nothing), `arm(holds: bool)`, `current() -> GateValue`, `set(open, generation: u64) -> Option<GateValue>` (Some when the value or the generation changed; the epoch moves by one); `GateValue { open: bool, epoch: u64, generation: Option<u64>, always_open: bool }`, `GateValue::open_for(self, current_generation: u64) -> bool`, `GateValue::allows_add(self, started_under: Option<u64>, current_generation: u64) -> bool`
  - `AppState.account_gate: AccountGate`
  - `StartPermit::AccountBlocked`, `EngineStart::AccountBlocked`
  - `finder_setup::error::app_code::ACCOUNT_BLOCKED = 8`
  - `finder_setup::core::Trigger::{AccountHold, AccountReady}` (`as_str`: `account_hold`, `account_ready`)
  - `finder_setup::driver::Event::AccountHold { ack: oneshot::Sender<()> }`, `FinderSetupHandle::account_hold(&self, timeout: Duration) -> Result<(), String>`
  - `lib.rs`: `async fn hold_for_account(state: &AppState)` (`#[allow(dead_code)]` until Task 11 calls it)

- [ ] **Step 1: The gate type, test first**

`src-tauri/src/account_view/gate.rs`:

```rust
//! Spec 2026-10-07 C-R10: the account gate, `{ open, epoch, generation }`. The account-view driver writes it for the
//! session generation it derived. Every engine start reads it under the engine slot (`authorize_engine_start`), and the
//! Finder reconciler's add reads it immediately before it acts (`finder_setup::macos_ports`). Both read it against the
//! session generation that is current at that moment: a generation the driver has not derived for yet is closed
//! (Checking), so the engine start a sign-in makes in the same command is held until the driver has derived for it.
//! Every change moves `epoch` on, so an add whose check began under another epoch does nothing. The gate only ever
//! holds: nothing here removes Finder or stops an engine.

use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateValue {
    pub open: bool,
    pub epoch: u64,
    /// The session generation `open` was derived for; `None` until the driver first sets it.
    pub generation: Option<u64>,
    /// Unarmed: the gate holds nothing. Linux (C-R10), and every unit test that builds an `AppState`.
    pub always_open: bool,
}

impl GateValue {
    /// Open for the session generation that is current now. A generation the driver has not derived for is closed.
    pub fn open_for(self, current_generation: u64) -> bool {
        todo!("Task 9 step 2")
    }

    /// May an add whose check began under `started_under` act now? Only while open for the current generation, and only
    /// under the epoch its check began under. A check with no recorded epoch never adds (fail closed).
    pub fn allows_add(self, started_under: Option<u64>, current_generation: u64) -> bool {
        todo!("Task 9 step 2")
    }
}

/// Shared by `AppState` and the driver (a clone is the same gate).
#[derive(Debug, Clone)]
pub struct AccountGate(Arc<Mutex<GateValue>>);

/// Unarmed, so every unit test that builds an `AppState` holds nothing. `setup()` arms it before the startup restore can
/// start an engine (plan Task 11).
impl Default for AccountGate {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(GateValue { open: false, epoch: 0, generation: None, always_open: true })))
    }
}

impl AccountGate {
    /// Arm the gate. `holds` is `false` on Linux, whose gate never holds (C-R10). Armed, the gate is closed for every
    /// generation the driver has not set it for.
    pub fn arm(&self, holds: bool) {
        self.lock().always_open = !holds;
    }

    pub fn current(&self) -> GateValue {
        *self.lock()
    }

    /// The driver's value for `generation`. The new value when `open` or the generation changed (the epoch moves on by
    /// one); `None` when both already are so (nothing moves).
    pub fn set(&self, open: bool, generation: u64) -> Option<GateValue> {
        todo!("Task 9 step 2")
    }

    fn lock(&self) -> MutexGuard<'_, GateValue> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed() -> AccountGate {
        let gate = AccountGate::default();
        gate.arm(true);
        gate
    }

    #[test]
    fn a_repeat_of_the_same_value_changes_nothing() {
        let gate = armed();
        assert_eq!(gate.current(), GateValue { open: false, epoch: 0, generation: None, always_open: false });
        assert_eq!(gate.set(true, 3), Some(GateValue { open: true, epoch: 1, generation: Some(3), always_open: false }));
        assert_eq!(gate.set(true, 3), None);
        assert_eq!(gate.current().epoch, 1);
    }

    #[test]
    fn every_change_of_value_or_generation_moves_the_epoch_on_by_one() {
        let gate = armed();
        gate.set(true, 1);
        assert_eq!(gate.set(false, 1).map(|v| (v.open, v.epoch)), Some((false, 2)));
        assert_eq!(gate.set(false, 1), None);
        assert_eq!(gate.set(true, 1).map(|v| (v.open, v.epoch)), Some((true, 3)));
        assert_eq!(gate.set(true, 2).map(|v| (v.generation, v.epoch)), Some((Some(2), 4)), "a new generation alone moves it");
    }

    /// Plan review I1: the engine start a sign-in makes in the same command finds the gate closed.
    #[test]
    fn a_generation_the_driver_has_not_derived_is_closed() {
        let gate = armed();
        assert!(!gate.current().open_for(1), "nothing derived yet");
        gate.set(true, 1);
        assert!(gate.current().open_for(1));
        assert!(!gate.current().open_for(2), "a sign-in moved the generation on; the driver has not derived for it");
        gate.set(false, 2);
        assert!(!gate.current().open_for(2));
    }

    #[test]
    fn an_unarmed_gate_holds_nothing() {
        let gate = AccountGate::default();
        assert!(gate.current().open_for(7));
        gate.set(false, 7);
        assert!(gate.current().open_for(7), "unarmed: the derived value is kept but never holds");
        let linux = AccountGate::default();
        linux.arm(false);
        linux.set(false, 1);
        assert!(linux.current().open_for(1), "Linux arms with holds = false");
    }

    #[test]
    fn an_add_needs_the_gate_open_for_this_generation_and_the_epoch_its_check_began_under() {
        let open = GateValue { open: true, epoch: 4, generation: Some(2), always_open: false };
        assert!(open.allows_add(Some(4), 2));
        assert!(!GateValue { open: false, ..open }.allows_add(Some(4), 2));
        assert!(!GateValue { epoch: 5, ..open }.allows_add(Some(4), 2), "closed and reopened since the check began");
        assert!(!open.allows_add(None, 2), "no recorded epoch: fail closed");
        assert!(!open.allows_add(Some(4), 3), "the generation moved on since the driver derived");
    }

    #[test]
    fn clones_share_one_gate() {
        let gate = armed();
        let driver_side = gate.clone();
        driver_side.set(true, 1);
        assert!(gate.current().open_for(1));
    }
}
```

Add `pub mod gate;` to `account_view/mod.rs`. Run with filter `account_view::gate`. Expected: `test result: FAILED. 0 passed; 6 failed` (every test reaches a `todo!()`).

- [ ] **Step 2: Implement the gate**

```rust
    pub fn open_for(self, current_generation: u64) -> bool {
        self.always_open || (self.open && self.generation == Some(current_generation))
    }

    pub fn allows_add(self, started_under: Option<u64>, current_generation: u64) -> bool {
        self.open_for(current_generation) && started_under == Some(self.epoch)
    }
```

```rust
    pub fn set(&self, open: bool, generation: u64) -> Option<GateValue> {
        let mut value = self.lock();
        if value.open == open && value.generation == Some(generation) {
            return None;
        }
        value.open = open;
        value.generation = Some(generation);
        value.epoch += 1;
        Some(*value)
    }
```

Run with filter `account_view::gate`. Expected: `test result: ok. 6 passed; 0 failed`.

- [ ] **Step 2b: Mutation-check the gate**

1. In `open_for`, drop `&& self.generation == Some(current_generation)`. Expected: `a_generation_the_driver_has_not_derived_is_closed` ("a sign-in moved the generation on…") and `an_add_needs_…` ("the generation moved on since the driver derived"). Revert.
2. In `set`, drop `&& value.generation == Some(generation)` from the early return. Expected: `every_change_of_value_or_generation_moves_the_epoch_on_by_one` ("a new generation alone moves it"). Revert.
3. In `set`, delete `value.epoch += 1;`. Expected: `a_repeat_of_the_same_value_changes_nothing` (left `epoch: 0`). Revert.
4. In `allows_add`, drop `&& started_under == Some(self.epoch)`. Expected: `an_add_needs_…` ("closed and reopened since the check began"). Revert.
5. In `arm`, write `always_open = holds`. Expected: `an_unarmed_gate_holds_nothing` ("Linux arms with holds = false") and `a_generation_the_driver_has_not_derived_is_closed` ("nothing derived yet"). Revert.
6. Replace `#[derive(Debug, Clone)]` on `AccountGate` with `#[derive(Debug)]` and a manual `impl Clone for AccountGate { fn clone(&self) -> Self { Self(Arc::new(Mutex::new(self.current()))) } }` (a copy, not the same gate). Expected: `clones_share_one_gate`. Revert.

Paste each failure and the green rerun.

- [ ] **Step 3: Write the failing reconciler tests**

First run the loop command with filter `finder_setup::` on the unchanged tree and note its `test result:` count (the before-count Step 4 compares with).

In `finder_setup/core.rs`'s `mod tests` (the module can read the private `held` field):

```rust
    // ── Spec 2026-10-07 C-R10: the account gate's hold ───────────────────────────────────────────

    fn signed_in() -> SessionFacts {
        SessionFacts { vault_unlocked: true, auth_present: true, ..SessionFacts::default() }
    }

    /// The result a well-behaved world gives each operation.
    fn answer(op: Op) -> OpResult {
        match op {
            Op::Observe => OpResult::Observed(Observation { facts: signed_in(), domain: Ok(DomainState::NotRegistered) }),
            Op::StartEngine => OpResult::EngineStarted(Ok(true)),
            Op::AddDomain => OpResult::Added(Ok(())),
            Op::ReadDomain => OpResult::Domain(Ok(DomainState::Enabled)),
            Op::WaitStable(_) => OpResult::Stable(Ok(())),
            Op::FinishReady => OpResult::Finished(Ok(())),
            Op::StopEngine => OpResult::EngineStopped(Ok(())),
            Op::RemoveDomain => OpResult::Removed(Ok(())),
        }
    }

    /// Answer every operation the core asks for until it is idle; returns the operations it ran.
    fn settle(state: &mut CoreState, now: Instant, policy: &RetryPolicy) -> Vec<Op> {
        let mut ran = Vec::new();
        for _ in 0..32 {
            match next_op(state, now, policy) {
                Next::Run(op) => {
                    ran.push(op);
                    *state = step(state.clone(), Input::Done(answer(op)), now, policy).0;
                }
                Next::WakeAt(_) | Next::Idle => return ran,
            }
        }
        panic!("the core never settled: {ran:?}");
    }

    fn blocked() -> FpError {
        FpError::app(app_code::ACCOUNT_BLOCKED, "the account is not ready for Finder")
    }

    #[test]
    fn an_account_blocked_engine_start_ends_the_check_with_no_failure_and_holds() {
        let (now, policy) = (Instant::now(), RetryPolicy::default());
        let (mut s, _) = step(CoreState::new(LaunchLocation::Applications), Input::Trigger(Trigger::Launch), now, &policy);
        assert_eq!(next_op(&mut s, now, &policy), Next::Run(Op::Observe));
        s = step(s, Input::Done(answer(Op::Observe)), now, &policy).0;
        assert_eq!(next_op(&mut s, now, &policy), Next::Run(Op::StartEngine));
        let (mut s, effects) = step(s, Input::Done(OpResult::EngineStarted(Err(blocked()))), now, &policy);
        assert_eq!((s.setup, s.reason), (FinderSetup::Missing, None), "no failure reason is shown");
        assert!(!effects.iter().any(|e| matches!(e, Effect::PersistFailure(Some(_)))), "{effects:?}");
        assert!(!effects.iter().any(|e| matches!(e, Effect::Transition(t) if t.to == FinderSetup::Failed)), "{effects:?}");
        assert!(s.held);
        assert_eq!(next_op(&mut s, now, &policy), Next::Idle);
    }

    #[test]
    fn an_account_blocked_add_ends_the_check_with_no_failure_and_holds() {
        let (now, policy) = (Instant::now(), RetryPolicy::default());
        let (mut s, _) = step(CoreState::new(LaunchLocation::Applications), Input::Trigger(Trigger::Launch), now, &policy);
        for op in [Op::Observe, Op::StartEngine] {
            assert_eq!(next_op(&mut s, now, &policy), Next::Run(op));
            s = step(s, Input::Done(answer(op)), now, &policy).0;
        }
        assert_eq!(next_op(&mut s, now, &policy), Next::Run(Op::AddDomain));
        let (mut s, effects) = step(s, Input::Done(OpResult::Added(Err(blocked()))), now, &policy);
        assert_eq!((s.setup, s.reason), (FinderSetup::Missing, None));
        assert!(!effects.iter().any(|e| matches!(e, Effect::PersistFailure(Some(_)))), "{effects:?}");
        assert!(s.held);
        assert_eq!(next_op(&mut s, now, &policy), Next::Idle);
    }

    #[test]
    fn a_check_in_flight_when_the_account_turns_blocking_adds_nothing() {
        let (now, policy) = (Instant::now(), RetryPolicy::default());
        let (mut s, _) = step(CoreState::new(LaunchLocation::Applications), Input::Trigger(Trigger::Launch), now, &policy);
        for op in [Op::Observe, Op::StartEngine] {
            assert_eq!(next_op(&mut s, now, &policy), Next::Run(op));
            s = step(s, Input::Done(answer(op)), now, &policy).0;
        }
        // The driver applies every event before it asks for the next operation, so the hold lands here.
        let (mut s, _) = step(s, Input::Trigger(Trigger::AccountHold), now, &policy);
        assert_eq!(next_op(&mut s, now, &policy), Next::Idle, "no AddDomain after the hold");
        let (mut s, _) = step(s, Input::Trigger(Trigger::TryAgain), now, &policy);
        assert_eq!(next_op(&mut s, now, &policy), Next::Idle, "a held reconciler ignores Try again");
    }

    #[test]
    fn blocking_never_removes_finder() {
        let (now, policy) = (Instant::now(), RetryPolicy::default());
        let (mut s, _) = step(CoreState::new(LaunchLocation::Applications), Input::Trigger(Trigger::Launch), now, &policy);
        settle(&mut s, now, &policy);
        assert_eq!(s.setup, FinderSetup::Ready);
        let (mut s, effects) = step(s, Input::Trigger(Trigger::AccountHold), now, &policy);
        assert!(!effects.iter().any(|e| matches!(e, Effect::RemoveFinished(_) | Effect::DomainRemovalConfirmed)), "{effects:?}");
        let mut ran = settle(&mut s, now, &policy);
        for trigger in [Trigger::TryAgain, Trigger::UserEnabledFlipped] {
            s = step(s, Input::Trigger(trigger), now, &policy).0;
            ran.extend(settle(&mut s, now, &policy));
        }
        assert!(!ran.contains(&Op::RemoveDomain), "{ran:?}");
        assert_eq!(s.setup, FinderSetup::Ready, "held, and still in Finder");
    }

    #[test]
    fn account_ready_clears_the_hold_and_requests_a_check() {
        let (now, policy) = (Instant::now(), RetryPolicy::default());
        let (mut s, _) = step(CoreState::new(LaunchLocation::Applications), Input::Trigger(Trigger::AccountHold), now, &policy);
        assert!(s.held);
        assert_eq!(next_op(&mut s, now, &policy), Next::Idle);
        let (mut s, effects) = step(s, Input::Trigger(Trigger::AccountReady), now, &policy);
        assert!(!s.held);
        assert!(effects.contains(&Effect::TriggerReceived(Trigger::AccountReady)));
        assert_eq!(next_op(&mut s, now, &policy), Next::Run(Op::Observe));
    }

    #[test]
    fn the_two_account_triggers_have_their_log_names() {
        assert_eq!(Trigger::AccountHold.as_str(), "account_hold");
        assert_eq!(Trigger::AccountReady.as_str(), "account_ready");
    }
```

(`FpError`, `app_code`, `LaunchLocation`, `RetryPolicy`, `Instant` are already imported at the top of `core.rs`; `Effect` derives `PartialEq`.)

Run with filter `finder_setup::core::tests`. Expected: compile errors `no variant named AccountHold` / `ACCOUNT_BLOCKED`. Paste them.

- [ ] **Step 4: Implement in the reconciler**

`finder_setup/error.rs`, in `app_code` after `ENGINE_STOP_UNCONFIRMED`:

```rust
    /// Spec 2026-10-07 C-R10: the account gate is closed (the account is being checked, needs an update, or has no
    /// plan yet), or changed since this check began. The core holds and shows no failure reason.
    pub const ACCOUNT_BLOCKED: i64 = 8;
```

`finder_setup/core.rs`:
- add `AccountHold,` and `AccountReady,` at the end of `enum Trigger`, and in `as_str`: `Self::AccountHold => "account_hold",` and `Self::AccountReady => "account_ready",`
- import `APP_DOMAIN`: `use super::error::{APP_DOMAIN, FpError, app_code};`
- in `on_trigger`, replace the `Trigger::Lock => { … }` arm and the `Trigger::Launch | Trigger::KeysArrived =>` pattern with:

```rust
        // Spec 2026-10-07 C-R10: the account gate closing holds like Lock (and is acknowledged the same way, before
        // the engine stops). Neither removes anything.
        Trigger::Lock | Trigger::AccountHold => hold(s, fx),
        Trigger::Launch | Trigger::KeysArrived | Trigger::AccountReady => {
            s.held = false;
            request_check(s, now);
        }
```

- move the old `Trigger::Lock` arm's body, unchanged (comments included), into:

```rust
/// Lock, the account gate's hold, and an add or engine start the account gate refused: cancel the check, hold, and
/// never remove. Only `Launch`, `KeysArrived` or `AccountReady` lifts the hold.
fn hold(s: &mut CoreState, fx: &mut Vec<Effect>) {
    s.held = true;
    s.poll_pending = false;
    s.check_again = false;
    if matches!(s.phase, Phase::Running(_) | Phase::Backoff(_)) {
        // (the old Lock arm's comment block, unchanged)
        s.phase = Phase::Idle;
        s.check_started = None;
        s.check_started_engine = false;
        if s.setup == FinderSetup::Adding {
            let reason = missing_reason(s.launch);
            set_state(s, FinderSetup::Missing, reason, None, fx);
        }
    }
}
```

- at the top of `on_check_result`:

```rust
    // Spec 2026-10-07 C-R10: the account gate refused this check's engine start, add or finish. That is a hold, not a
    // failure: no reason, no retry, nothing persisted.
    if account_blocked(&result) {
        hold(s, fx);
        return;
    }
```

with

```rust
fn account_blocked(result: &OpResult) -> bool {
    let error = match result {
        OpResult::EngineStarted(Err(error)) | OpResult::Added(Err(error)) | OpResult::Finished(Err(error)) => error,
        _ => return false,
    };
    error.domain == APP_DOMAIN && error.code == app_code::ACCOUNT_BLOCKED
}
```

`finder_setup/driver.rs`: add to `enum Event`:

```rust
    /// Spec 2026-10-07 C-R10: the account gate closed. `ack` resolves once any check is cancelled and the hold is set.
    AccountHold { ack: oneshot::Sender<()> },
```

in `Driver::event`:

```rust
            Event::AccountHold { ack } => {
                self.input(Input::Trigger(Trigger::AccountHold), ports, clock);
                let _ = ack.send(());
            }
```

and on `FinderSetupHandle`, after `lock`:

```rust
    /// Spec 2026-10-07 C-R10: hold for the account gate. Resolves once any check is cancelled and the hold is set;
    /// `Trigger::AccountReady` lifts it. Never removes the domain.
    pub async fn account_hold(&self, timeout: Duration) -> Result<(), String> {
        let (ack, done) = oneshot::channel();
        self.tx.send(Event::AccountHold { ack }).map_err(|_| NOT_RUNNING.to_string())?;
        match tokio::time::timeout(timeout, done).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("the Finder reconciler stopped".to_string()),
            Err(_) => Err(format!("the Finder reconciler did not acknowledge the account hold within {}s", timeout.as_secs())),
        }
    }
```

If a test or helper matches `Event` exhaustively, add the arm the compiler names. If a lifecycle-log test pins the list of trigger names, add `account_hold` and `account_ready` to it.

Run with filter `finder_setup::`. Expected: `test result: ok. N passed; 0 failed` with N = the before-count from Step 3 + 6. Paste the line.

- [ ] **Step 5: Mutation-check the reconciler**

1. Delete the `if account_blocked(&result) { … }` block. Expected: `an_account_blocked_engine_start_ends_the_check_with_no_failure_and_holds` and `an_account_blocked_add_…` fail (a `Failed` transition or a persisted failure). Revert.
2. Route `Trigger::AccountHold` to the `Trigger::SignOut` arm. Expected: `blocking_never_removes_finder` (`RemoveDomain` ran). Revert.
3. Drop `| Trigger::AccountReady` from the clearing arm (send it to the `TryAgain` arm). Expected: `account_ready_clears_the_hold_and_requests_a_check`. Revert.
4. Send `Trigger::AccountHold` to the `TryAgain` arm instead of `hold`. Expected: `a_check_in_flight_when_the_account_turns_blocking_adds_nothing` ("no AddDomain after the hold"). Revert.
5. Swap the two `as_str` names. Expected: `the_two_account_triggers_have_their_log_names`. Revert.

- [ ] **Step 6: The add's guard in `macos_ports.rs`**

Add the field and capture the epoch at each `Observe` (every check, and every retry, starts with one):

```rust
pub struct MacosPorts {
    app: tauri::AppHandle,
    gate: BridgeGate,
    /// Spec 2026-10-07 C-R10: the account gate's epoch when the current check observed.
    check_epoch: Option<u64>,
}
```

`new` sets `check_epoch: None`. In `fn run(&mut self, op: Op)`, before `async move`:

```rust
        if op == Op::Observe {
            self.check_epoch = Some(app.state::<AppState>().account_gate.current().epoch);
        }
        let check_epoch = self.check_epoch;
```

and directly above the existing `Op::AddDomain =>` arm:

```rust
                // Spec 2026-10-07 C-R10: the account gate at the point of action. Closed, or changed since this check
                // observed: add nothing; the core holds with no failure reason.
                Op::AddDomain if !app.state::<AppState>().account_gate.current().allows_add(check_epoch, crate::session_generation()) => {
                    OpResult::Added(Err(FpError::app(app_code::ACCOUNT_BLOCKED, "the account is not ready for Finder")))
                }
```

Add to `macos_ports.rs`'s tests:

```rust
    /// Spec 2026-10-07 C-R10: the add reads the gate, with the epoch its check observed under and the session generation
    /// current at that moment, before the bridge call.
    #[test]
    fn the_add_reads_the_account_gate_under_the_epoch_of_its_observe() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/finder_setup/macos_ports.rs")).unwrap();
        let run = &source[source.find("fn run(&mut self, op: Op)").expect("run")..];
        let capture = run.find("if op == Op::Observe {").expect("the epoch is captured at Observe");
        let guard = run.find("Op::AddDomain if !").expect("a guarded add arm");
        let add = run.find("Op::AddDomain =>").expect("the add arm");
        assert!(capture < guard && guard < add, "capture, then the guard, then the real add");
        assert!(run[guard..add].contains("allows_add(check_epoch,"), "{}", &run[guard..add]);
        assert!(!run[guard..add].contains("macos_file_provider::add_domain"), "the guard never calls the bridge");
    }
```

- [ ] **Step 7: Write the failing engine-start tests in `lib.rs`**

In the R10 test module that holds `another_accounts_engine_starts_only_after_the_local_state_was_purged`:

```rust
    /// Spec 2026-10-07 C-R10: a closed account gate starts nothing and binds nothing; open again, the start goes on.
    #[test]
    fn a_closed_account_gate_starts_nothing_and_binds_nothing() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let local = local_data_of(Some(alice()));
            let fx = AuthorizeFixture::with_session(&bob());
            fx.state.account_gate.arm(true);
            assert!(fx.state.account_gate.set(false, crate::session_generation()).is_some());
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let called = std::cell::Cell::new(false);
            let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                called.set(true);
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            assert_eq!(result, Ok(EngineStart::AccountBlocked));
            assert!(!called.get() && slot.is_none(), "no engine was created");
            assert_eq!(local.data.db.owner().unwrap(), Some(alice()), "nothing was bound or reset");
            assert_eq!(local.data.db.list_due_operations(i64::MAX).unwrap().len(), 1, "the queue is untouched");
            let result = start_open(&fx, &mut slot, &paths);
            assert_ne!(result, Ok(EngineStart::AccountBlocked), "the open gate lets the start go on to the binding");
        });
    }

    /// Opens the gate for the current generation and starts. Other lib tests may move a process-wide generation between
    /// the two reads; a start whose generation moved underneath it is repeated (at most three times).
    fn start_open(
        fx: &AuthorizeFixture,
        slot: &mut tokio::sync::MutexGuard<'_, Option<crate::runner::EngineRunner>>,
        paths: &LocalDataPaths,
    ) -> Result<EngineStart, String> {
        let mut result = Err("not run".to_string());
        for _ in 0..3 {
            let now = crate::session_generation();
            fx.state.account_gate.set(true, now);
            result = start_engine_bound(&fx.state, &fx.acct, slot, paths, |_root, _token, _key| {
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            if crate::session_generation() == now {
                break;
            }
        }
        result
    }

    /// Plan review I1 (lead ruling 2026-10-07): a first sign-in of an account that turns out to be blocking starts no
    /// engine. A sign-in moves the session generation on before its own engine start (Task 0 recorded the order); the
    /// driver last set the gate for the signed-out generation before it, open (Signed out holds nothing). The start the
    /// sign-in makes in the same command must find the gate closed; the driver's Ready starts the engine later.
    #[test]
    fn a_first_sign_in_with_a_blocking_account_starts_no_engine() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let local = local_data_of(None);
            let fx = AuthorizeFixture::with_session(&bob());
            fx.state.account_gate.arm(true);
            let signed_out_generation = crate::session_generation().wrapping_sub(1);
            assert!(fx.state.account_gate.set(true, signed_out_generation).is_some(), "Signed out: open, for the old generation");
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let called = std::cell::Cell::new(false);
            let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                called.set(true);
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            assert_eq!(result, Ok(EngineStart::AccountBlocked), "the driver has not derived for this generation: Checking");
            assert!(!called.get() && slot.is_none(), "no engine was created");
            assert_eq!(local.data.db.owner().unwrap(), None, "nothing was bound");
        });
    }
```

In `finder_setup_command_tests` (it has `production_source` and `body_between`):

```rust
    /// Spec 2026-10-07 C-R10: every caller of `spawn_bound_engine` treats `AccountBlocked` as held. Only the
    /// reconciler's start turns it into the code the core holds on, and its reused-engine branch reads the gate too.
    #[test]
    fn every_spawn_bound_engine_caller_treats_account_blocked_as_held() {
        let production = production_source();
        for caller in [
            "async fn start_engine_if_possible(",
            "async fn ensure_sync_root_and_engine(",
            "async fn persist_sync_root_and_start_engine(",
            "async fn pick_sync_root(",
            "async fn start_engine_for_pending_finder_install(",
            "async fn start_check_engine(",
        ] {
            let body = body_between(&production, caller, "\n}\n");
            assert!(body.contains("EngineStart::AccountBlocked"), "{caller} must name AccountBlocked");
        }
        let check = body_between(&production, "async fn start_check_engine(", "\n}\n");
        let reuse = check.find("if let Some(existing) = engine_slot.as_ref()").expect("the reused-engine branch");
        let gate = check[reuse..].find("account_gate.current().open_for(").expect("the reused engine reads the gate");
        let handle = check[reuse..].find("existing.ipc_bind_error_handle()").expect("the reuse");
        assert!(gate < handle, "the gate is read before the running engine is reused");
        assert!(check.contains("app_code::ACCOUNT_BLOCKED"));
        let authorize = body_between(&production, "fn authorize_engine_start(", "\n}\n");
        let gate = authorize.find("account_gate.current().open_for(").expect("authorize reads the gate for the current generation");
        let bind = authorize.find("bind_local_data_to_session(").expect("the binding");
        assert!(gate < bind, "a closed gate returns before anything is bound");
    }

    /// Spec 2026-10-07 C-R10: closing the gate holds the reconciler first, then stops a running engine through the one
    /// stop helper, and never removes Finder.
    #[test]
    fn closing_the_account_gate_holds_then_stops_and_never_removes() {
        let production = production_source();
        let hold = body_between(&production, "async fn hold_for_account(", "\n}\n");
        let stop = hold.find("stop_engine_in_slot(").expect("the one stop helper");
        let ack = hold.find(".account_hold(").expect("the reconciler is held");
        assert!(ack < stop, "held before the engine stops");
        for removal in ["finder_remove_for", "Trigger::SignOut", "Trigger::Repair", "RemoveDomain", "macos_file_provider::remove"] {
            assert!(!hold.contains(removal), "{removal} in hold_for_account");
        }
    }
```

Run with the filter `-- a_closed_account_gate_starts_nothing_and_binds_nothing a_first_sign_in_with_a_blocking_account_starts_no_engine every_spawn_bound_engine_caller_treats_account_blocked_as_held closing_the_account_gate_holds_then_stops_and_never_removes`. Expected: compile errors (`no field account_gate`, `no variant AccountBlocked`), then — once those compile — assertion failures. Paste them.

- [ ] **Step 8: Implement the engine-start side in `lib.rs`**

`AppState`, after `finder_setup`:

```rust
    /// Spec 2026-10-07 C-R10: the account gate every engine start reads under the engine slot. The account-view driver
    /// writes it.
    pub account_gate: account_view::gate::AccountGate,
```

and in `impl Default for AppState`: `account_gate: account_view::gate::AccountGate::default(),`.

`EngineStart`, after `FinderRemovalOwed`:

```rust
    /// Spec 2026-10-07 C-R10: the account gate is closed (the account is being checked, needs an update, or has no plan
    /// yet). Nothing was started or bound. Every caller treats it as held, never as a failure.
    AccountBlocked,
```

`StartPermit`: add `AccountBlocked,` and in its `Debug`: `Self::AccountBlocked => f.write_str("AccountBlocked"),`.

`authorize_engine_start`, directly after the `let (keys, identity) = match keys_for_engine_start(acct) { … };` statement:

```rust
    // Spec 2026-10-07 C-R10: the account gate, read under the engine slot for the session generation current now. A
    // locked vault answered `NoSession` above; a closed gate, or a generation the account view has not derived for yet,
    // returns before anything is bound, adopted or reset.
    if !state.account_gate.current().open_for(crate::session_generation()) {
        return Ok(StartPermit::AccountBlocked);
    }
```

`start_engine_bound`: `StartPermit::AccountBlocked => return Ok(EngineStart::AccountBlocked),`.

The six callers:

1. `start_engine_if_possible`, in its `match spawn_bound_engine(…)?`:

```rust
            EngineStart::AccountBlocked => {
                tracing::info!("engine start held: the account gate is closed");
            }
```

2. `ensure_sync_root_and_engine` (the sync folder is already saved above the match):

```rust
            // Spec 2026-10-07 C-R10: held, never a failure; the folder stays saved.
            EngineStart::AccountBlocked => {}
```

3. `persist_sync_root_and_start_engine`: replace `spawn_bound_engine(app, state, &acct, &mut engine_slot, root)?;` with

```rust
        // Spec 2026-10-07 C-R10: a closed account gate holds the start; the root is saved either way.
        if spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? == EngineStart::AccountBlocked {
            tracing::info!("engine start held: the account gate is closed");
        }
```

4. `pick_sync_root`: the same replacement for its `spawn_bound_engine(…)?;` statement (keep its own argument names).

5. `start_engine_for_pending_finder_install`: replace `if spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? == EngineStart::NoSession { return Err(…); }` with

```rust
            match spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? {
                EngineStart::NoSession => return Err("Unlock the vault before installing the Finder location.".to_string()),
                // Spec 2026-10-07 C-R10: held; the install goes on and the engine starts when the account is ready.
                EngineStart::AccountBlocked => {
                    tracing::info!("engine start held: the account gate is closed");
                    return Ok(false);
                }
                EngineStart::Started | EngineStart::FinderRemovalOwed => {}
            }
```

(keep the error string exactly as it is in the code at HEAD).

6. `start_check_engine`: in the reused-engine branch,

```rust
        if let Some(existing) = engine_slot.as_ref() {
            // Spec 2026-10-07 C-R10: a running engine is reused only while the account gate is open for this generation.
            if !state.account_gate.current().open_for(crate::session_generation()) {
                return Err(FpError::app(app_code::ACCOUNT_BLOCKED, ACCOUNT_HELD_FOR_FINDER));
            }
            (false, existing.ipc_bind_error_handle())
```

and in its `match spawn_bound_engine(…)` add

```rust
                EngineStart::AccountBlocked => {
                    return Err(FpError::app(app_code::ACCOUNT_BLOCKED, ACCOUNT_HELD_FOR_FINDER));
                }
```

with, beside `FINDER_REMOVAL_OWED_ERROR`:

```rust
/// Spec 2026-10-07 C-R10: the reconciler's start found the account gate closed. The core holds on this code and shows
/// no reason, so this text only reaches the lifecycle log.
#[cfg(target_os = "macos")]
const ACCOUNT_HELD_FOR_FINDER: &str = "The account is not ready for Finder yet.";
```

Then the closing helper, after `stop_engine_in_slot`:

```rust
/// Spec 2026-10-07 C-R10: the account gate just closed. The Finder reconciler is held first (its acknowledgement means
/// no check is in flight and none starts), then a running engine is stopped through the one stop helper, which keeps
/// its unconfirmed-stop semantics. Finder is never removed: the gate only holds adds.
// The account view calls this from the app's ports (plan Task 11, which removes this allow).
#[allow(dead_code)]
async fn hold_for_account(state: &AppState) {
    #[cfg(target_os = "macos")]
    {
        if let Some(handle) = state.finder_setup.get()
            && let Err(error) = handle.account_hold(ACCOUNT_HOLD_ACK_LIMIT).await
        {
            tracing::warn!(%error, "account gate: the Finder reconciler did not acknowledge the hold");
        }
    }
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    let Ok(acct) = state.active_account() else {
        return;
    };
    let mut engine_slot = acct.engine.lock().await;
    if let Some(engine) = engine_slot.take()
        && !stop_engine_in_slot(&acct, engine).await.is_stopped()
    {
        tracing::warn!("account gate: the sync engine did not confirm it stopped");
    }
}

/// How long the account gate waits for the reconciler's acknowledgement (one operation's longest limit, with margin).
#[cfg(target_os = "macos")]
const ACCOUNT_HOLD_ACK_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);
```

Per `t0-record.md`: if `SESSION_TRANSITION` is on every platform, drop the `#[cfg(target_os = "windows")]` on the `_transition` line so the stop is serialized with sign-in and sign-out everywhere; if it is Windows-only, keep it as shown. (The `#[cfg]` on the macOS hold wraps a block, not the `if let` itself, so the attribute sits on a statement every edition accepts.)

- [ ] **Step 9: Run the tests to see them pass**

Run with the four names from Step 7, then the whole lib under the scratch-HOME recipe (Global Constraints).
Expected: `test result: ok. 4 passed; 0 failed` for the four; `every_engine_start_is_bound_first` (spec A) still passes with six callers; on macOS the lib's `test result:` is `B_lib` + 100 (the running total in Global Constraints). Spec A tests that build an `AppState` with `Default` keep passing because the default gate is unarmed.

- [ ] **Step 10: Mutation-check**

1. Move the gate check in `authorize_engine_start` below `bind_local_data_to_session`. Expected: `a_closed_account_gate_starts_nothing_and_binds_nothing` (the owner or the queue changed) and the source test. Revert.
2. Delete the reused-engine gate check. Expected: `every_spawn_bound_engine_caller_treats_account_blocked_as_held`. Revert.
3. In `hold_for_account`, put the stop before the hold. Expected: `closing_the_account_gate_holds_then_stops_and_never_removes`. Revert.
4. In `macos_ports.rs`, replace `allows_add(check_epoch, crate::session_generation())` with `open`. Expected: `the_add_reads_the_account_gate_under_the_epoch_of_its_observe`. Revert.
5. In `authorize_engine_start`, replace `.open_for(crate::session_generation())` with `.open`. Expected: `a_first_sign_in_with_a_blocking_account_starts_no_engine` (left `Ok(Started)` or another non-blocked value) and `every_spawn_bound_engine_caller_treats_account_blocked_as_held` ("authorize reads the gate for the current generation"). Revert.

- [ ] **Step 11: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t9-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/gate.rs src-tauri/src/account_view/mod.rs src-tauri/src/lib.rs src-tauri/src/finder_setup/error.rs src-tauri/src/finder_setup/core.rs src-tauri/src/finder_setup/driver.rs src-tauri/src/finder_setup/macos_ports.rs
git commit -m "desktop: hold engine starts and the Finder add while the account is not ready" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/gate.rs src-tauri/src/account_view/mod.rs src-tauri/src/lib.rs src-tauri/src/finder_setup/error.rs src-tauri/src/finder_setup/core.rs src-tauri/src/finder_setup/driver.rs src-tauri/src/finder_setup/macos_ports.rs
git show --stat HEAD
```

---

## Task 10: The driver — one task that fetches, binds, caches, derives and acts (`account_view::driver`)

**Lane R.** Async, generic over `Ports` and `Clock`; the tests run the real loop with fake ports on a manual clock. Also adds the three lifecycle events (§10).

**Files:**
- Create: `src-tauri/src/account_view/driver.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod driver;`)
- Modify: `src-tauri/src/lifecycle_log.rs` (`LifecycleEvent` + `format_line` + a test)

**Interfaces:**
- Consumes: `derive::{derive, AccountState, AccountView, Blocking, Derived, FirstFetch, Inputs, TokenState}` (Task 5, `Derived.condition` included), `doc::{AccountPart, Doc, Stage}` (Task 2), `fetch::{FetchOutcome, UnavailableKind}` (Task 8), `gate::AccountGate` (Task 9), `policy::{self, Outcome, Schedule}` (Task 6), `account_binding::{same_account, Identity}` (spec A), `finder_setup::driver::Clock` (spec A: `now() -> Instant`, `unix_now() -> i64`, `sleep_until(Instant)`), `lifecycle_log::LifecycleEvent` (spec A).
- Produces (`crate::account_view::driver`):
  - `ACCOUNT_VIEW_CHANGED_EVENT = "account-view-changed"`
  - `struct Facts { token_stored: bool, auth_expired: bool, owner_recorded: bool, key_in_memory: bool, key_in_keychain: bool, identity: Identity, generation: u64 }` (Default)
  - `enum CacheRead { Row { identity: Identity, part: AccountPart }, Empty, Unreadable }`
  - `enum Event { Launch { restoring: bool }, RestoreFinished, SessionChanged, WindowFocused, PlanOpened, Retry }`
  - `enum HeldReason { Checking, AccountBlocking, UpdateRequired }`, `enum DocumentOutcome { UnavailableHttp, Unreadable, UnknownSchema, BlockingVerifyEmail, BlockingUnknownStep }` (both with `as_str`)
  - `enum AccountLog { State { from, to, offline }, EngineHeld(HeldReason), Document(DocumentOutcome) }` with `lifecycle_event(self) -> LifecycleEvent` and `line(self) -> String`
  - `trait Ports` (below), `trait Publisher: Send + Sync + 'static { fn publish(&self, view: &AccountView); }` (the event and the menu-bar icon: Task 11's `AppPublisher`), `struct AccountViewHandle` with `send(Event)`, `state() -> AccountView`, `clear_for_sign_out()`, `settled(deadline: tokio::time::Instant) -> AccountView`, `for_test(view)`
  - `fn start<P: Ports, C: Clock>(ports, clock, gate: AccountGate, publisher: Arc<dyn Publisher>, initial: AccountView) -> (AccountViewHandle, impl Future<Output = ()> + Send)` (publishes `initial` before it returns), `fn spawn(...) -> AccountViewHandle` (same parameters, on Tauri's runtime)
  - **Every view change goes through one publish** (plan review C1, lead ruling 2026-10-07): the driver's derived views and the sign-out/switch clear alike. It compares with the last *published* view, sets the watch channel, then calls the `Publisher`; nothing else writes the watch channel.
  - `lifecycle_log::LifecycleEvent::{AccountState { from: &'static str, to: &'static str, offline: bool }, EngineHeld { reason: &'static str }, OnboardingDocument { outcome: &'static str }}`

```rust
pub trait Ports: Send + 'static {
    fn platform(&self) -> Platform;
    fn facts(&self) -> Facts;
    /// Only the session generation (cheap; read under the publish lock).
    fn generation(&self) -> u64;
    /// `bearer`: send the session's token (signed in) or nothing (signed out).
    fn fetch(&mut self, bearer: bool) -> impl Future<Output = FetchOutcome> + Send;
    fn read_cache(&mut self) -> CacheRead;
    fn write_cache(&mut self, identity: &Identity, part: &AccountPart);
    fn delete_cache(&mut self);
    /// Spec A's `AuthHealth::note_result` for a signed-in fetch: `true` for a 401, `false` for a usable document.
    fn note_auth(&mut self, unauthorized: bool);
    fn log(&mut self, entry: AccountLog);
    /// The gate closed: hold the reconciler, then stop a running engine (`hold_for_account`).
    fn gate_closed(&mut self) -> impl Future<Output = ()> + Send;
    /// C-R12: macOS sends `Trigger::AccountReady`; Windows and Linux start the engine when none runs.
    fn account_ready(&mut self) -> impl Future<Output = ()> + Send;
}
```

- [ ] **Step 1: Add the three lifecycle events, test first**

In `lifecycle_log.rs`, at the end of `enum LifecycleEvent`:

```rust
    /// Spec 2026-10-07 §10: the account state changed. `from`/`to` are `AccountState::as_str` names.
    AccountState { from: &'static str, to: &'static str, offline: bool },
    /// Spec 2026-10-07 §10: the account gate closed. `reason`: `checking`, `account_blocking` or `update_required`.
    EngineHeld { reason: &'static str },
    /// Spec 2026-10-07 §10: the onboarding document was not usable, or is blocking without a screen (C-R15).
    OnboardingDocument { outcome: &'static str },
```

and in `format_line`'s `match`:

```rust
        LifecycleEvent::AccountState { from, to, offline } => {
            format!("account_state from={} to={} offline={}", token(from), token(to), if *offline { "yes" } else { "no" })
        }
        LifecycleEvent::EngineHeld { reason } => format!("engine_held reason={}", token(reason)),
        LifecycleEvent::OnboardingDocument { outcome } => format!("onboarding_document outcome={}", token(outcome)),
```

Test, in `lifecycle_log.rs`'s tests:

```rust
    #[test]
    fn the_account_lines_use_the_closed_vocabulary() {
        let line = |event: LifecycleEvent| format_line(&event, &KnownNames::new(), std::time::UNIX_EPOCH);
        assert_eq!(
            line(LifecycleEvent::AccountState { from: "checking", to: "no_plan", offline: true }),
            "1970-01-01T00:00:00Z account_state from=checking to=no_plan offline=yes"
        );
        assert_eq!(line(LifecycleEvent::EngineHeld { reason: "account_blocking" }), "1970-01-01T00:00:00Z engine_held reason=account_blocking");
        assert_eq!(
            line(LifecycleEvent::OnboardingDocument { outcome: "unavailable_http" }),
            "1970-01-01T00:00:00Z onboarding_document outcome=unavailable_http"
        );
    }
```

(Write the test first, run it with filter `the_account_lines_use_the_closed_vocabulary` and paste the compile error; then add the variants and arms; run again: `test result: ok. 1 passed`.) If any exhaustive `match` on `LifecycleEvent` elsewhere fails to compile, add the three arms the compiler names in the same style.

- [ ] **Step 2: Write `driver.rs`: types, the handle and the loop**

```rust
//! Spec 2026-10-07 §5 and §13 (`account_view::driver`): the one task. It fetches the onboarding document on the
//! schedule (`policy`), drops every result fetched for an earlier session (C-D5), keeps one cached account part per
//! account, derives the `AccountView` (`derive`), publishes it through the one publish path, and acts on the account
//! gate, which it sets for the session generation it derived: when it closes, the
//! Finder reconciler is held and a running engine stops; every transition into Ready fires the account trigger
//! (C-R12). Generic over `Ports` and `Clock`, so the tests run the whole loop on a manual clock. The document body is
//! never logged (C-D6): only the closed vocabulary of `AccountLog` is.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{mpsc, watch};

use super::derive::{self, AccountState, AccountView, Blocking, Derived, FirstFetch, Inputs, TokenState};
use super::doc::{AccountPart, Doc, Stage};
use super::fetch::{FetchOutcome, UnavailableKind};
use super::gate::AccountGate;
use super::policy::{self, Outcome, Schedule};
use crate::account_binding::{self, Identity};
use crate::finder_setup::driver::Clock;
use crate::lifecycle_log::LifecycleEvent;
use crate::surfaces::policy::Platform;

pub const ACCOUNT_VIEW_CHANGED_EVENT: &str = "account-view-changed";

/// What the app knows about the session right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// A session token is stored (`AppState::auth_present`).
    pub token_stored: bool,
    /// Spec A's `AuthHealth::is_expired()`.
    pub auth_expired: bool,
    /// Spec A's owner record exists (`StateDb::owner()` is `Some`; unreadable counts as recorded).
    pub owner_recorded: bool,
    pub key_in_memory: bool,
    pub key_in_keychain: bool,
    /// The session's identity (spec A `identity_of_session`); empty with no session.
    pub identity: Identity,
    /// Spec A Task 12's session generation (plan "Spec issues" 15).
    pub generation: u64,
}

pub enum CacheRead {
    Row { identity: Identity, part: AccountPart },
    Empty,
    /// No database, one that cannot be read, or a row this version cannot read.
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The app started. `restoring`: a stored token's restore and probe are running (`RestoreFinished` ends them).
    Launch { restoring: bool },
    RestoreFinished,
    /// A sign-in, sign-out, switch, lock or unlock happened, or spec A's auth-expired state may have changed.
    SessionChanged,
    WindowFocused,
    /// "Choose a plan on beebeeb.io" was clicked: poll every `poll_seconds` for 15 minutes.
    PlanOpened,
    /// "Try again".
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldReason {
    Checking,
    AccountBlocking,
    UpdateRequired,
}

impl HeldReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::AccountBlocking => "account_blocking",
            Self::UpdateRequired => "update_required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOutcome {
    UnavailableHttp,
    Unreadable,
    UnknownSchema,
    BlockingVerifyEmail,
    BlockingUnknownStep,
}

impl DocumentOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnavailableHttp => "unavailable_http",
            Self::Unreadable => "unreadable",
            Self::UnknownSchema => "unknown_schema",
            Self::BlockingVerifyEmail => "blocking_verify_email",
            Self::BlockingUnknownStep => "blocking_unknown_step",
        }
    }
}

/// The three lifecycle lines of §10, each written once per change. macOS writes them to the lifecycle log; Windows
/// and Linux to `tracing` only (`line`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountLog {
    State { from: AccountState, to: AccountState, offline: bool },
    EngineHeld(HeldReason),
    Document(DocumentOutcome),
}

impl AccountLog {
    pub fn lifecycle_event(self) -> LifecycleEvent {
        match self {
            Self::State { from, to, offline } => LifecycleEvent::AccountState { from: from.as_str(), to: to.as_str(), offline },
            Self::EngineHeld(reason) => LifecycleEvent::EngineHeld { reason: reason.as_str() },
            Self::Document(outcome) => LifecycleEvent::OnboardingDocument { outcome: outcome.as_str() },
        }
    }

    /// The same words for `tracing`, without the timestamp.
    pub fn line(self) -> String {
        match self {
            Self::State { from, to, offline } => {
                format!("account_state from={} to={} offline={}", from.as_str(), to.as_str(), if offline { "yes" } else { "no" })
            }
            Self::EngineHeld(reason) => format!("engine_held reason={}", reason.as_str()),
            Self::Document(outcome) => format!("onboarding_document outcome={}", outcome.as_str()),
        }
    }
}

pub trait Ports: Send + 'static {
    fn platform(&self) -> Platform;
    fn facts(&self) -> Facts;
    /// Only the session generation (cheap; read under the publish lock).
    fn generation(&self) -> u64;
    /// `bearer`: send the session's token (signed in) or nothing (signed out).
    fn fetch(&mut self, bearer: bool) -> impl Future<Output = FetchOutcome> + Send;
    fn read_cache(&mut self) -> CacheRead;
    fn write_cache(&mut self, identity: &Identity, part: &AccountPart);
    fn delete_cache(&mut self);
    /// Spec A's `AuthHealth::note_result` for a signed-in fetch: `true` for a 401, `false` for a usable document.
    fn note_auth(&mut self, unauthorized: bool);
    fn log(&mut self, entry: AccountLog);
    /// The gate closed: hold the reconciler, then stop a running engine (`hold_for_account`).
    fn gate_closed(&mut self) -> impl Future<Output = ()> + Send;
    /// C-R12: macOS sends `Trigger::AccountReady`; Windows and Linux start the engine when none runs.
    fn account_ready(&mut self) -> impl Future<Output = ()> + Send;
}

/// Where a published view goes besides the watch channel: the `account-view-changed` event and the menu-bar icon
/// (Task 11's `AppPublisher`). It is called under the publish lock, so it must not block (post to the main thread,
/// never wait for it).
pub trait Publisher: Send + Sync + 'static {
    fn publish(&self, view: &AccountView);
}

/// The one way a view becomes visible (plan review C1). It compares with the last view it published, never with the
/// watch channel, so a clear followed by the driver's own Signed out publishes exactly once.
struct Published {
    view: watch::Sender<AccountView>,
    last: Mutex<Option<AccountView>>,
    publisher: Arc<dyn Publisher>,
}

impl Published {
    fn new(initial: AccountView, publisher: Arc<dyn Publisher>) -> Arc<Self> {
        Arc::new(Self { view: watch::channel(initial).0, last: Mutex::new(None), publisher })
    }

    /// Publish `view` unless `still_current()` says its session has ended or it equals the last published view.
    /// `true` when it published.
    fn publish(&self, view: AccountView, still_current: impl FnOnce() -> bool) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !still_current() || last.as_ref() == Some(&view) {
            return false;
        }
        self.view.send_replace(view.clone());
        self.publisher.publish(&view);
        *last = Some(view);
        true
    }
}

/// The handle every command, window and wiring point uses. Cloning it shares the one task.
#[derive(Clone)]
pub struct AccountViewHandle {
    tx: mpsc::UnboundedSender<Event>,
    published: Arc<Published>,
}

impl AccountViewHandle {
    pub fn send(&self, event: Event) {
        if self.tx.send(event).is_err() {
            tracing::warn!(?event, "account view: the task is not running");
        }
    }

    pub fn state(&self) -> AccountView {
        self.published.view.borrow().clone()
    }

    /// Called inside a sign-out by choice or an account switch, after the session generation moved on (C-D5): the old
    /// account's document, notice and links are gone before the transition returns, and the cleared view is published
    /// like any other (windows re-render, the menu-bar icon changes). The task re-derives right after.
    pub fn clear_for_sign_out(&self) {
        let offline = self.state().offline;
        self.published.publish(AccountView::signed_out(offline), || true);
        self.send(Event::SessionChanged);
    }

    /// C-W4: the view once it has left Checking, or at `deadline`, whichever comes first.
    pub async fn settled(&self, deadline: tokio::time::Instant) -> AccountView {
        let mut changes = self.published.view.subscribe();
        let left_checking = changes.wait_for(|view| view.state != AccountState::Checking);
        match tokio::time::timeout_at(deadline, left_checking).await {
            Ok(Ok(view)) => view.clone(),
            Ok(Err(_)) | Err(_) => self.state(),
        }
    }

    /// A handle whose view only changes when the test says so, plus the receiving end of its events.
    #[cfg(test)]
    pub fn for_test(view: AccountView) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx, published: Published::new(view, Arc::new(NoPublisher)) }, rx)
    }
}

#[cfg(test)]
struct NoPublisher;

#[cfg(test)]
impl Publisher for NoPublisher {
    fn publish(&self, _view: &AccountView) {}
}

struct Driver {
    gate: AccountGate,
    published: Arc<Published>,
    schedule: Schedule,
    launched: bool,
    restoring: bool,
    /// Plan review I5: Checking ends at the latest here (a launch or a new generation set it; `None` once it fired).
    checking_until: Option<Instant>,
    generation: Option<u64>,
    fresh: Option<Doc>,
    first_fetch: FirstFetch,
    cached: Option<AccountPart>,
    cache_loaded_for: Option<Identity>,
    unpersisted: bool,
    last_state: Option<AccountState>,
    last_offline: bool,
    last_blocking: Blocking,
    last_document: Option<DocumentOutcome>,
    last_held: Option<HeldReason>,
    fetch_now: bool,
    token_stored: bool,
}

pub fn start<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    gate: AccountGate,
    publisher: Arc<dyn Publisher>,
    initial: AccountView,
) -> (AccountViewHandle, impl Future<Output = ()> + Send) {
    let (tx, rx) = mpsc::unbounded_channel();
    let published = Published::new(initial.clone(), publisher);
    // The launch view goes out through the same path as every later one (the menu-bar icon included).
    published.publish(initial, || true);
    let driver = Driver {
        gate,
        published: published.clone(),
        schedule: Schedule::default(),
        launched: false,
        restoring: false,
        checking_until: None,
        generation: None,
        fresh: None,
        first_fetch: FirstFetch::Pending,
        cached: None,
        cache_loaded_for: None,
        unpersisted: false,
        last_state: None,
        last_offline: false,
        last_blocking: Blocking::None,
        last_document: None,
        last_held: None,
        fetch_now: false,
        token_stored: false,
    };
    (AccountViewHandle { tx, published }, run(driver, ports, clock, rx))
}

/// `start`, on Tauri's runtime.
pub fn spawn<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    gate: AccountGate,
    publisher: Arc<dyn Publisher>,
    initial: AccountView,
) -> AccountViewHandle {
    let (handle, task) = start(ports, clock, gate, publisher, initial);
    tauri::async_runtime::spawn(task);
    handle
}

async fn run<P: Ports, C: Clock>(mut d: Driver, mut ports: P, clock: C, mut rx: mpsc::UnboundedReceiver<Event>) {
    loop {
        // Cooperative: a manual clock must not starve the runtime.
        tokio::task::yield_now().await;
        while let Ok(event) = rx.try_recv() {
            d.event(event, &clock);
        }
        if !d.launched {
            match rx.recv().await {
                Some(event) => d.event(event, &clock),
                None => return,
            }
            continue;
        }
        d.watchdog(clock.now());
        d.refresh(&mut ports, clock.now()).await;
        if d.restoring {
            // No fetch until the restore has said whether the stored token still works, or until the watchdog.
            let watchdog = d.checking_until;
            tokio::select! {
                biased;
                event = rx.recv() => match event {
                    Some(event) => d.event(event, &clock),
                    None => return,
                },
                () = async {
                    match watchdog {
                        Some(at) => clock.sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => {}
            }
            continue;
        }
        let now = clock.now();
        let due = d.schedule.next_fetch_at(now, d.ttl(), d.poll());
        if d.fetch_now || due <= now {
            d.fetch_now = false;
            d.fetch(&mut ports, &clock).await;
            continue;
        }
        tokio::select! {
            biased;
            event = rx.recv() => match event {
                Some(event) => d.event(event, &clock),
                None => return,
            },
            // A due fetch, the offline re-probe included (which is also the wake re-probe, plan "Spec issues" 21).
            () = clock.sleep_until(due) => {}
        }
    }
}

impl Driver {
    fn event<C: Clock>(&mut self, event: Event, clock: &C) {
        match event {
            Event::Launch { restoring } => {
                self.launched = true;
                self.restoring = restoring;
                self.fetch_now = !restoring;
                self.checking_until = Some(policy::checking_deadline(clock.now()));
            }
            Event::RestoreFinished => {
                // Only a launch that waited for the restore fetches now; a signed-out launch already did.
                if std::mem::take(&mut self.restoring) {
                    self.fetch_now = true;
                }
            }
            // `refresh` re-reads the facts; a new generation fetches at once.
            Event::SessionChanged => {}
            Event::WindowFocused => {
                if self.schedule.focus_wants_fetch(clock.now(), self.poll()) {
                    self.fetch_now = true;
                }
            }
            Event::PlanOpened => {
                self.schedule.plan_opened(clock.now());
                self.fetch_now = true;
            }
            Event::Retry => self.fetch_now = true,
        }
    }

    fn part(&self) -> Option<&AccountPart> {
        self.fresh.as_ref().and_then(Doc::account_part).or(self.cached.as_ref())
    }

    fn ttl(&self) -> std::time::Duration {
        match &self.fresh {
            Some(doc) => policy::ttl(doc.ttl_seconds(), doc.stage()),
            None => policy::ttl(None, if self.token_stored { Stage::Account } else { Stage::PreAccount }),
        }
    }

    fn poll(&self) -> std::time::Duration {
        policy::poll(self.part().and_then(|part| part.poll_seconds))
    }

    /// Plan review I5 (lead ruling 2026-10-07): Checking ends at the latest 15 s after it began, even when the restore
    /// never reports back. That concludes "document unavailable", which fails open like any failed first fetch (the
    /// table then gives Ready with the key, Locked without it, Signed out without a session), and fetches at once.
    fn watchdog(&mut self, now: Instant) {
        let Some(until) = self.checking_until else {
            return;
        };
        if now < until {
            return;
        }
        self.checking_until = None;
        if self.restoring || self.first_fetch == FirstFetch::Pending {
            self.restoring = false;
            self.first_fetch = FirstFetch::Concluded;
            self.fetch_now = true;
        }
    }

    /// C-D5: the cached part is used only for the same account; another account's row is deleted, never shown; a row
    /// that cannot be compared is neither used nor deleted.
    fn load_cache<P: Ports>(ports: &mut P, identity: &Identity) -> Option<AccountPart> {
        match ports.read_cache() {
            CacheRead::Row { identity: owner, part } => match account_binding::same_account(&owner, identity) {
                Some(true) => Some(part),
                Some(false) => {
                    ports.delete_cache();
                    None
                }
                None => None,
            },
            CacheRead::Empty | CacheRead::Unreadable => None,
        }
    }

    async fn refresh<P: Ports>(&mut self, ports: &mut P, now: Instant) {
        let facts = ports.facts();
        self.token_stored = facts.token_stored;
        if self.generation != Some(facts.generation) {
            self.generation = Some(facts.generation);
            // A new session gets its own 15 s of Checking (plan review I5).
            self.checking_until = Some(policy::checking_deadline(now));
            self.fresh = None;
            self.first_fetch = FirstFetch::Pending;
            self.cached = None;
            self.cache_loaded_for = None;
            self.unpersisted = false;
            self.schedule = Schedule::default();
            self.last_document = None;
            self.fetch_now = !self.restoring;
        }
        if facts.token_stored {
            if self.cache_loaded_for.as_ref() != Some(&facts.identity) {
                self.cached = Self::load_cache(ports, &facts.identity);
                self.cache_loaded_for = Some(facts.identity.clone());
            }
            // C-D5: an unidentified session's document stays in memory until its identity is known.
            if self.unpersisted
                && facts.identity.user_id.is_some()
                && let Some(part) = self.fresh.as_ref().and_then(Doc::account_part)
            {
                ports.write_cache(&facts.identity, part);
                self.unpersisted = false;
            }
        } else {
            self.cached = None;
            self.cache_loaded_for = None;
        }
        let token = match (facts.token_stored, self.restoring) {
            (false, _) => TokenState::Absent,
            (true, true) => TokenState::Restoring,
            (true, false) => TokenState::Stored,
        };
        let derived = derive::derive(&Inputs {
            platform: ports.platform(),
            token,
            auth_expired: facts.auth_expired,
            owner_recorded: facts.owner_recorded,
            key_in_memory: facts.key_in_memory,
            key_in_keychain: facts.key_in_keychain,
            fresh: self.fresh.as_ref(),
            cached: self.cached.as_ref(),
            first_fetch: self.first_fetch,
            offline: self.schedule.offline(),
        });
        self.apply(derived, facts.generation, ports).await;
    }

    async fn apply<P: Ports>(&mut self, derived: Derived, generation: u64, ports: &mut P) {
        let state = derived.view.state;
        let offline = derived.view.offline;
        // C-R10: the gate first, so no start or add slips through while the rest of this runs. Closing it holds the
        // reconciler and stops a running engine once; the `engine_held` line is written whenever its reason changes.
        let held = (!derived.gate_open).then(|| match derived.condition {
            AccountState::UpdateRequired => HeldReason::UpdateRequired,
            AccountState::NoPlan => HeldReason::AccountBlocking,
            _ => HeldReason::Checking,
        });
        // The value is for the generation this view was derived for; any other generation reads closed (plan review I1).
        let changed = self.gate.set(derived.gate_open, generation);
        let closed_now = changed.is_some_and(|value| !value.open);
        let opened_now = changed.is_some_and(|value| value.open);
        if held != self.last_held {
            if let Some(reason) = held {
                ports.log(AccountLog::EngineHeld(reason));
            }
            self.last_held = held;
        }
        if closed_now {
            ports.gate_closed().await;
        }
        // C-R15: one line when a document turns blocking without a screen.
        if derived.blocking != self.last_blocking {
            match derived.blocking {
                Blocking::VerifyEmail => ports.log(AccountLog::Document(DocumentOutcome::BlockingVerifyEmail)),
                Blocking::UnknownStep => ports.log(AccountLog::Document(DocumentOutcome::BlockingUnknownStep)),
                Blocking::None | Blocking::NeedsPlan => {}
            }
            self.last_blocking = derived.blocking;
        }
        if let Some(from) = self.last_state
            && (from != state || self.last_offline != offline)
        {
            ports.log(AccountLog::State { from, to: state, offline });
        }
        // C-R12: every transition into Ready fires the account trigger, at launch from the cache included. So does a
        // gate that opened for a new session generation while Ready: that generation's own engine start was held.
        let fire_ready = state == AccountState::Ready && (self.last_state != Some(AccountState::Ready) || opened_now);
        self.last_state = Some(state);
        self.last_offline = offline;
        // C-D5 and plan review C1: the one publish path. A view derived for a session that has since ended is never
        // published over the cleared one, and a view equal to the last published one is not published again.
        self.published.publish(derived.view, || ports.generation() == generation);
        if fire_ready {
            ports.account_ready().await;
        }
    }

    async fn fetch<P: Ports, C: Clock>(&mut self, ports: &mut P, clock: &C) {
        let before = ports.facts();
        let bearer = before.token_stored;
        let outcome = ports.fetch(bearer).await;
        let now = clock.now();
        let after = ports.facts();
        // C-D5: a result fetched for another session is dropped, never applied or cached.
        if after.generation != before.generation {
            return;
        }
        let ttl = self.ttl();
        self.first_fetch = FirstFetch::Concluded;
        match outcome {
            FetchOutcome::Document(doc) => {
                if bearer {
                    ports.note_auth(false);
                }
                self.schedule.record(Outcome::Document, now, policy::ttl(doc.ttl_seconds(), doc.stage()));
                self.last_document = None;
                if let Some(part) = doc.account_part() {
                    if after.identity.user_id.is_some() {
                        ports.write_cache(&after.identity, part);
                        self.unpersisted = false;
                    } else {
                        self.unpersisted = true;
                    }
                }
                self.fresh = Some(doc);
            }
            FetchOutcome::Unauthorized => {
                if bearer {
                    ports.note_auth(true);
                }
                self.schedule.record(Outcome::Answered, now, ttl);
            }
            FetchOutcome::RateLimited { retry_after } => {
                self.schedule.record(Outcome::RateLimited(retry_after), now, ttl);
            }
            FetchOutcome::Unavailable(kind) => {
                self.schedule.record(Outcome::Answered, now, ttl);
                let outcome = match kind {
                    UnavailableKind::Http(_) => DocumentOutcome::UnavailableHttp,
                    UnavailableKind::Unreadable => DocumentOutcome::Unreadable,
                    UnavailableKind::UnknownSchema => DocumentOutcome::UnknownSchema,
                };
                if self.last_document != Some(outcome) {
                    ports.log(AccountLog::Document(outcome));
                    self.last_document = Some(outcome);
                }
            }
            FetchOutcome::Network => {
                self.schedule.record(Outcome::Network, now, ttl);
            }
        }
    }
}
```

- [ ] **Step 3: Write the driver tests**

Append to `driver.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::doc::tests::{edited, fixture};
    use crate::account_view::doc::parse;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct World {
        platform: Option<Platform>,
        facts: Facts,
        responses: VecDeque<FetchOutcome>,
        /// When set, a fetch waits for it before answering (to act while a fetch is in flight).
        hold_fetch: Option<Arc<Notify>>,
        /// When set, closing the gate waits for it (to act while the driver is between deriving and publishing).
        hold_gate: Option<Arc<Notify>>,
        fetches: Vec<bool>,
        cache: Option<(Identity, AccountPart)>,
        writes: Vec<Identity>,
        deletes: usize,
        auth: Vec<bool>,
        published: Vec<AccountView>,
        logs: Vec<AccountLog>,
        gate_closed: usize,
        ready: usize,
    }

    #[derive(Clone)]
    struct FakePorts(Arc<Mutex<World>>);

    impl Ports for FakePorts {
        fn platform(&self) -> Platform {
            self.0.lock().unwrap().platform.unwrap_or(Platform::Macos)
        }
        fn facts(&self) -> Facts {
            self.0.lock().unwrap().facts.clone()
        }
        fn generation(&self) -> u64 {
            self.0.lock().unwrap().facts.generation
        }
        fn fetch(&mut self, bearer: bool) -> impl Future<Output = FetchOutcome> + Send {
            let world = self.0.clone();
            async move {
                let hold = {
                    let mut w = world.lock().unwrap();
                    w.fetches.push(bearer);
                    w.hold_fetch.clone()
                };
                if let Some(hold) = hold {
                    hold.notified().await;
                }
                world.lock().unwrap().responses.pop_front().unwrap_or(FetchOutcome::Network)
            }
        }
        fn read_cache(&mut self) -> CacheRead {
            match self.0.lock().unwrap().cache.clone() {
                Some((identity, part)) => CacheRead::Row { identity, part },
                None => CacheRead::Empty,
            }
        }
        fn write_cache(&mut self, identity: &Identity, part: &AccountPart) {
            let mut w = self.0.lock().unwrap();
            w.writes.push(identity.clone());
            w.cache = Some((identity.clone(), part.clone()));
        }
        fn delete_cache(&mut self) {
            let mut w = self.0.lock().unwrap();
            w.deletes += 1;
            w.cache = None;
        }
        fn note_auth(&mut self, unauthorized: bool) {
            self.0.lock().unwrap().auth.push(unauthorized);
        }
        fn log(&mut self, entry: AccountLog) {
            self.0.lock().unwrap().logs.push(entry);
        }
        fn gate_closed(&mut self) -> impl Future<Output = ()> + Send {
            let world = self.0.clone();
            async move {
                let hold = {
                    let mut w = world.lock().unwrap();
                    w.gate_closed += 1;
                    w.hold_gate.clone()
                };
                if let Some(hold) = hold {
                    hold.notified().await;
                }
            }
        }
        fn account_ready(&mut self) -> impl Future<Output = ()> + Send {
            let world = self.0.clone();
            async move { world.lock().unwrap().ready += 1 }
        }
    }

    /// Records every published view, the launch view included.
    struct FakePublisher(Arc<Mutex<World>>);

    impl Publisher for FakePublisher {
        fn publish(&self, view: &AccountView) {
            self.0.lock().unwrap().published.push(view.clone());
        }
    }

    /// Time moves only when the test says so; `sleep_until` waits for that.
    #[derive(Clone)]
    struct ManualClock {
        now: Arc<Mutex<Instant>>,
        wall: Arc<Mutex<i64>>,
        moved: Arc<Notify>,
    }

    impl ManualClock {
        fn new() -> Self {
            Self { now: Arc::new(Mutex::new(Instant::now())), wall: Arc::new(Mutex::new(1_791_000_000)), moved: Arc::new(Notify::new()) }
        }
        fn advance(&self, by: Duration) {
            *self.now.lock().unwrap() += by;
            *self.wall.lock().unwrap() += by.as_secs() as i64;
            self.moved.notify_waiters();
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
        fn unix_now(&self) -> i64 {
            *self.wall.lock().unwrap()
        }
        fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send {
            let clock = self.clone();
            async move {
                loop {
                    let moved = clock.moved.notified();
                    if clock.now() >= at {
                        return;
                    }
                    moved.await;
                }
            }
        }
    }

    struct Harness {
        world: Arc<Mutex<World>>,
        clock: ManualClock,
        handle: AccountViewHandle,
        gate: AccountGate,
    }

    impl Harness {
        fn new(world: World) -> Self {
            let world = Arc::new(Mutex::new(world));
            let clock = ManualClock::new();
            let gate = AccountGate::default();
            gate.arm(true);
            let publisher = Arc::new(FakePublisher(world.clone()));
            let (handle, task) = start(FakePorts(world.clone()), clock.clone(), gate.clone(), publisher, AccountView::signed_out(false));
            tokio::spawn(task);
            Self { world, clock, handle, gate }
        }

        async fn until(&self, what: &str, done: impl Fn(&World, &AccountView) -> bool) {
            for _ in 0..500 {
                if done(&self.world.lock().unwrap(), &self.handle.state()) {
                    return;
                }
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            panic!("never: {what}; view {:?}", self.handle.state());
        }

        fn with(&self, change: impl FnOnce(&mut World)) {
            change(&mut self.world.lock().unwrap());
            self.handle.send(Event::SessionChanged);
        }

        fn respond(&self, outcome: FetchOutcome) {
            self.world.lock().unwrap().responses.push_back(outcome);
        }

        fn fetches(&self) -> usize {
            self.world.lock().unwrap().fetches.len()
        }

        fn ready_count(&self) -> usize {
            self.world.lock().unwrap().ready
        }
    }

    fn sam() -> Identity {
        Identity::new(Some("u-1"), Some("sam@beebeeb.io"))
    }

    fn signed_in(generation: u64) -> Facts {
        Facts { token_stored: true, owner_recorded: true, key_in_memory: true, key_in_keychain: true, identity: sam(), generation, ..Facts::default() }
    }

    fn document(name: &str) -> FetchOutcome {
        FetchOutcome::Document(parse(&fixture(name)).unwrap())
    }

    fn update_required() -> FetchOutcome {
        FetchOutcome::Document(parse(&edited("account.active.web.json", |v| v["client"]["status"] = serde_json::json!("update_required"))).unwrap())
    }

    fn part(name: &str) -> AccountPart {
        parse(&fixture(name)).unwrap().account_part().cloned().unwrap()
    }

    /// Signed in with the key, launched, restore finished, the first fetch answered with `first`.
    async fn ready_with(first: FetchOutcome) -> Harness {
        let h = Harness::new(World { facts: signed_in(1), ..World::default() });
        h.respond(first);
        h.handle.send(Event::Launch { restoring: true });
        h.handle.send(Event::RestoreFinished);
        h.until("the first fetch", |w, _| w.fetches.len() == 1).await;
        h
    }

    #[tokio::test]
    async fn a_launch_without_a_session_fetches_anonymously_and_shows_sign_in() {
        let h = Harness::new(World::default());
        h.respond(document("pre_account.desktop.json"));
        h.handle.send(Event::Launch { restoring: false });
        h.until("an anonymous fetch", |w, v| w.fetches == [false] && v.state == AccountState::SignedOut).await;
        assert_eq!(h.handle.state().screen, derive::Screen::SignIn);
        assert!(h.gate.current().open);
        assert_eq!(h.world.lock().unwrap().auth, Vec::<bool>::new(), "an anonymous fetch never reports to AuthHealth");
        h.handle.send(Event::RestoreFinished);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.fetches(), 1, "the restore's end after a signed-out launch fetches nothing more");
    }

    #[tokio::test]
    async fn a_stored_token_waits_for_the_restore_then_fetches_with_the_bearer() {
        let h = Harness::new(World { facts: signed_in(1), ..World::default() });
        h.with(|w| w.facts.key_in_memory = false);
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Launch { restoring: true });
        h.until("Checking during the restore", |_, v| v.state == AccountState::Checking).await;
        assert!(!h.gate.current().open);
        assert_eq!(h.fetches(), 0, "no fetch while the restore runs");
        h.until("held for checking", |w, _| w.gate_closed == 1 && w.logs.contains(&AccountLog::EngineHeld(HeldReason::Checking))).await;
        h.with(|w| w.facts.key_in_memory = true);
        h.handle.send(Event::RestoreFinished);
        h.until("Ready after the document", |w, v| w.fetches == [true] && v.state == AccountState::Ready).await;
        assert!(h.gate.current().open);
        assert_eq!(h.ready_count(), 1);
        assert_eq!(h.world.lock().unwrap().auth, [false], "a usable signed-in document reports success");
    }

    #[tokio::test]
    async fn a_result_fetched_for_an_earlier_session_is_dropped_and_never_cached() {
        let hold = Arc::new(Notify::new());
        let h = Harness::new(World { facts: signed_in(1), hold_fetch: Some(hold.clone()), ..World::default() });
        h.respond(document("account.needs_plan.ios.json"));
        h.handle.send(Event::Launch { restoring: false });
        h.until("the fetch is in flight", |w, _| w.fetches.len() == 1).await;
        h.world.lock().unwrap().facts.generation = 2;
        h.world.lock().unwrap().hold_fetch = None;
        hold.notify_waiters();
        h.until("a fetch for the new session", |w, _| w.fetches.len() >= 2).await;
        let w = h.world.lock().unwrap();
        assert!(w.writes.is_empty(), "the earlier session's document was never cached");
        assert!(!w.published.iter().any(|v| v.state == AccountState::NoPlan), "and never shown");
    }

    #[tokio::test]
    async fn update_required_is_never_carried_by_the_cache() {
        let h = ready_with(update_required()).await;
        h.until("Update required", |_, v| v.state == AccountState::UpdateRequired).await;
        assert_eq!(h.world.lock().unwrap().writes.len(), 1, "the account part was cached");
        h.with(|w| w.facts.generation = 2);
        h.until("the next session reads the cache and fetches", |w, _| w.fetches.len() == 2).await;
        h.until("Ready from the cache, never Update required", |_, v| v.state == AccountState::Ready).await;
    }

    #[tokio::test]
    async fn an_unidentified_session_keeps_its_document_in_memory_until_its_identity_is_known() {
        let h = Harness::new(World { facts: Facts { identity: Identity::new(None, Some("sam@beebeeb.io")), ..signed_in(1) }, ..World::default() });
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Launch { restoring: false });
        h.until("Ready", |_, v| v.state == AccountState::Ready).await;
        assert!(h.world.lock().unwrap().writes.is_empty(), "no user id: memory only");
        h.with(|w| w.facts.identity = sam());
        h.until("persisted once the identity is known", |w, _| w.writes == [sam()]).await;
    }

    #[tokio::test]
    async fn a_cache_row_that_cannot_be_compared_is_kept_and_not_used() {
        let h = Harness::new(World { facts: signed_in(1), cache: Some((Identity::default(), part("account.needs_plan.ios.json"))), ..World::default() });
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Launch { restoring: false });
        h.until("the fetch concluded", |w, v| w.fetches.len() == 1 && v.state == AccountState::Ready).await;
        let w = h.world.lock().unwrap();
        assert_eq!(w.deletes, 0, "cannot compare: never deleted");
        assert!(w.cache.is_some());
    }

    #[tokio::test]
    async fn another_accounts_cache_row_is_deleted_and_never_shown() {
        let kim = Identity::new(Some("u-2"), Some("kim@beebeeb.io"));
        let h = Harness::new(World { facts: signed_in(1), cache: Some((kim, part("account.needs_plan.ios.json"))), ..World::default() });
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Launch { restoring: false });
        h.until("the fetch concluded", |w, _| w.fetches.len() == 1).await;
        h.until("Ready, not the other account's No plan", |_, v| v.state == AccountState::Ready).await;
        let w = h.world.lock().unwrap();
        assert_eq!(w.deletes, 1);
        assert!(!w.published.iter().any(|v| v.state == AccountState::NoPlan));
    }

    #[tokio::test]
    async fn account_ready_fires_at_launch_from_the_cache() {
        let h = Harness::new(World { facts: signed_in(1), cache: Some((sam(), part("account.active.web.json"))), ..World::default() });
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Launch { restoring: true });
        h.handle.send(Event::RestoreFinished);
        h.until("Ready from the cache", |w, v| v.state == AccountState::Ready && w.ready == 1).await;
        h.until("the fetch ran", |w, _| w.fetches.len() == 1).await;
        assert_eq!(h.ready_count(), 1, "a failed fetch afterwards does not fire again");
    }

    #[tokio::test]
    async fn account_ready_fires_from_checking() {
        let h = ready_with(document("account.active.web.json")).await;
        h.until("Ready", |w, v| v.state == AccountState::Ready && w.ready == 1).await;
    }

    #[tokio::test]
    async fn account_ready_fires_from_no_plan() {
        let h = ready_with(document("account.needs_plan.ios.json")).await;
        h.until("No plan", |_, v| v.state == AccountState::NoPlan).await;
        assert_eq!(h.ready_count(), 0);
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Retry);
        h.until("Ready", |w, v| v.state == AccountState::Ready && w.ready == 1).await;
    }

    #[tokio::test]
    async fn account_ready_fires_from_locked() {
        let h = ready_with(document("account.active.web.json")).await;
        h.until("Ready", |_, v| v.state == AccountState::Ready).await;
        h.with(|w| w.facts.key_in_memory = false);
        h.until("Locked", |_, v| v.state == AccountState::Locked).await;
        h.with(|w| w.facts.key_in_memory = true);
        h.until("Ready again", |w, v| v.state == AccountState::Ready && w.ready == 2).await;
    }

    #[tokio::test]
    async fn account_ready_fires_from_update_required() {
        let h = ready_with(update_required()).await;
        h.until("Update required", |_, v| v.state == AccountState::UpdateRequired).await;
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Retry);
        h.until("Ready", |w, v| v.state == AccountState::Ready && w.ready == 1).await;
    }

    #[tokio::test]
    async fn account_ready_fires_from_session_ended() {
        let h = ready_with(document("account.active.web.json")).await;
        h.until("Ready", |_, v| v.state == AccountState::Ready).await;
        h.with(|w| w.facts.auth_expired = true);
        h.until("Session ended", |_, v| v.state == AccountState::SessionEnded).await;
        h.with(|w| w.facts.auth_expired = false);
        h.until("Ready after the auth-expired state cleared", |w, v| v.state == AccountState::Ready && w.ready == 2).await;
    }

    #[tokio::test]
    async fn account_ready_fires_on_no_other_edge() {
        let h = ready_with(document("account.active.web.json")).await;
        h.until("Ready", |w, _| w.ready == 1).await;
        // Ready to Ready: a refetch with the same answer, and the offline overlay coming and going.
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Retry);
        h.until("the refetch", |w, _| w.fetches.len() == 2).await;
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Retry);
        h.until("a network failure", |w, _| w.fetches.len() == 3).await;
        h.clock.advance(Duration::from_secs(10));
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Retry);
        h.until("offline", |_, v| v.offline).await;
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::Retry);
        h.until("online", |_, v| !v.offline).await;
        // Out of Ready: to No plan, to Locked, to Signed out.
        h.respond(document("account.needs_plan.ios.json"));
        h.handle.send(Event::Retry);
        h.until("No plan", |_, v| v.state == AccountState::NoPlan).await;
        h.with(|w| w.facts.key_in_memory = false);
        h.until("Locked", |_, v| v.state == AccountState::Locked).await;
        h.with(|w| w.facts = Facts { generation: 9, ..Facts::default() });
        h.until("Signed out", |_, v| v.state == AccountState::SignedOut).await;
        assert_eq!(h.ready_count(), 1, "only the first transition into Ready fired");
    }

    #[tokio::test]
    async fn closing_the_gate_holds_once_and_says_why() {
        let h = ready_with(document("account.needs_plan.ios.json")).await;
        h.until("No plan", |w, v| v.state == AccountState::NoPlan && w.gate_closed == 1).await;
        assert!(!h.gate.current().open);
        h.respond(document("account.needs_plan.ios.json"));
        h.handle.send(Event::Retry);
        h.until("the refetch", |w, _| w.fetches.len() == 2).await;
        let w = h.world.lock().unwrap();
        assert_eq!(w.gate_closed, 1, "a gate that stays closed is not closed again");
        assert_eq!(w.logs.iter().filter(|l| matches!(l, AccountLog::EngineHeld(HeldReason::AccountBlocking))).count(), 1);
    }

    #[tokio::test]
    async fn the_lifecycle_lines_are_written_once_per_change() {
        let h = ready_with(FetchOutcome::Unavailable(UnavailableKind::Http(503))).await;
        h.respond(FetchOutcome::Unavailable(UnavailableKind::Http(503)));
        h.handle.send(Event::Retry);
        h.until("two 503s", |w, _| w.fetches.len() == 2).await;
        let verify = parse(&edited("account.active.web.json", |v| {
            v["blocking"] = serde_json::json!(true);
            v["steps"] = serde_json::json!([{ "id": "verify_email", "status": "todo", "required": true, "ui": "action" }]);
        }))
        .unwrap();
        h.respond(FetchOutcome::Document(verify.clone()));
        h.handle.send(Event::Retry);
        h.until("the verify_email document", |w, _| w.fetches.len() == 3).await;
        h.respond(FetchOutcome::Document(verify));
        h.handle.send(Event::Retry);
        h.until("again", |w, _| w.fetches.len() == 4).await;
        let logs = h.world.lock().unwrap().logs.clone();
        let count = |entry: AccountLog| logs.iter().filter(|l| **l == entry).count();
        assert_eq!(count(AccountLog::Document(DocumentOutcome::UnavailableHttp)), 1, "{logs:?}");
        assert_eq!(count(AccountLog::Document(DocumentOutcome::BlockingVerifyEmail)), 1, "{logs:?}");
        assert_eq!(count(AccountLog::State { from: AccountState::Checking, to: AccountState::Ready, offline: false }), 1, "{logs:?}");
    }

    #[tokio::test]
    async fn a_401_reports_to_auth_health_and_ends_checking() {
        let h = ready_with(FetchOutcome::Unauthorized).await;
        h.until("Ready (unavailable: proceed as today)", |w, v| w.auth == [true] && v.state == AccountState::Ready).await;
    }

    #[tokio::test]
    async fn a_429_ends_checking_as_unavailable() {
        let h = ready_with(FetchOutcome::RateLimited { retry_after: Some(Duration::from_secs(20)) }).await;
        h.until("Ready", |_, v| v.state == AccountState::Ready).await;
        assert!(h.gate.current().open);
    }

    /// Review Focus 2, plan "Spec issues" 5.
    #[tokio::test]
    async fn a_first_network_failure_ends_checking_without_showing_offline() {
        let h = ready_with(FetchOutcome::Network).await;
        h.until("Ready after one failure", |_, v| v.state == AccountState::Ready).await;
        assert!(!h.handle.state().offline);
        assert!(h.gate.current().open);
    }

    /// Review Focus 4: C-R13.
    #[tokio::test]
    async fn an_offline_relaunch_keeps_the_cached_gate_closed() {
        let h = Harness::new(World { facts: signed_in(1), cache: Some((sam(), part("account.needs_plan.ios.json"))), ..World::default() });
        h.respond(FetchOutcome::Network);
        h.handle.send(Event::Launch { restoring: true });
        h.handle.send(Event::RestoreFinished);
        h.until("the first failure", |w, v| w.fetches.len() == 1 && v.state == AccountState::NoPlan).await;
        assert!(!h.gate.current().open);
        h.clock.advance(Duration::from_secs(10));
        h.until("the 10 s retry", |w, _| w.fetches.len() == 2).await;
        h.until("offline", |_, v| v.offline && v.state == AccountState::NoPlan).await;
        assert!(!h.gate.current().open, "never opened while offline");
        assert_eq!(h.ready_count(), 0);
    }

    #[tokio::test]
    async fn focus_fetches_only_after_poll_seconds() {
        let h = ready_with(document("account.active.web.json")).await;
        h.handle.send(Event::WindowFocused);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.fetches(), 1, "the last fetch is younger than poll_seconds");
        h.clock.advance(Duration::from_secs(4));
        h.respond(document("account.active.web.json"));
        h.handle.send(Event::WindowFocused);
        h.until("a focus fetch", |w, _| w.fetches.len() == 2).await;
    }

    #[tokio::test]
    async fn plan_opened_polls_every_3s() {
        let h = ready_with(document("account.needs_plan.ios.json")).await;
        h.respond(document("account.needs_plan.ios.json"));
        h.handle.send(Event::PlanOpened);
        h.until("at once", |w, _| w.fetches.len() == 2).await;
        h.respond(document("account.needs_plan.ios.json"));
        h.clock.advance(Duration::from_secs(3));
        h.until("3 s later", |w, _| w.fetches.len() == 3).await;
        h.respond(document("account.active.web.json"));
        h.clock.advance(Duration::from_secs(3));
        h.until("continues by itself", |w, v| w.fetches.len() == 4 && v.state == AccountState::Ready && w.ready == 1).await;
    }

    #[tokio::test]
    async fn offline_reprobes_every_30s_on_the_clock() {
        let h = ready_with(FetchOutcome::Network).await;
        h.clock.advance(Duration::from_secs(10));
        h.until("the second failure", |w, v| w.fetches.len() == 2 && v.offline).await;
        h.clock.advance(Duration::from_secs(29));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.fetches(), 2);
        h.clock.advance(Duration::from_secs(1));
        h.until("the 30 s probe", |w, _| w.fetches.len() == 3).await;
    }

    #[tokio::test]
    async fn a_sign_out_clears_the_view_inside_the_transition() {
        let h = ready_with(document("account.trialing.desktop.json")).await;
        h.until("Ready with a notice", |_, v| v.notice.is_some()).await;
        h.world.lock().unwrap().facts = Facts { generation: 2, ..Facts::default() };
        h.handle.clear_for_sign_out();
        let view = h.handle.state();
        assert_eq!((view.state, view.notice.as_ref()), (AccountState::SignedOut, None), "cleared before anything awaits");
        assert_eq!(view.links, crate::account_view::links::Links::built_in());
    }

    /// C-D5: the driver is between deriving (for the session that is ending) and publishing when the sign-out lands.
    #[tokio::test]
    async fn a_view_derived_for_an_ended_session_never_overwrites_the_cleared_one() {
        let hold = Arc::new(Notify::new());
        let h = ready_with(document("account.trialing.desktop.json")).await;
        h.until("Ready", |w, v| v.state == AccountState::Ready && w.gate_closed == 1).await;
        h.world.lock().unwrap().hold_gate = Some(hold.clone());
        h.respond(document("account.needs_plan.ios.json"));
        h.handle.send(Event::Retry);
        h.until("the gate is closing for No plan", |w, _| w.gate_closed == 2).await;
        h.world.lock().unwrap().facts = Facts { generation: 2, ..Facts::default() };
        h.handle.clear_for_sign_out();
        h.world.lock().unwrap().hold_gate = None;
        hold.notify_waiters();
        h.until("the signed-out session's fetch", |w, _| w.fetches.len() >= 3).await;
        assert_eq!(h.handle.state().state, AccountState::SignedOut);
        assert!(!h.world.lock().unwrap().published.iter().any(|v| v.state == AccountState::NoPlan), "the ended session's view was never published");
    }

    /// Plan review C1: a live sign-out publishes Signed out inside the transition (the windows re-render and the
    /// menu-bar icon changes from that publish; Task 14 adds the icon's assertion), and the driver's own Signed out
    /// afterwards is not published a second time.
    #[tokio::test]
    async fn a_live_sign_out_publishes_signed_out() {
        let h = ready_with(document("account.trialing.desktop.json")).await;
        h.until("Ready with a notice", |_, v| v.notice.is_some()).await;
        h.world.lock().unwrap().facts = Facts { generation: 2, ..Facts::default() };
        h.handle.clear_for_sign_out();
        let last = h.world.lock().unwrap().published.last().cloned().expect("a published view");
        assert_eq!((last.state, last.notice.as_ref()), (AccountState::SignedOut, None), "published inside the transition");
        h.until("the signed-out fetch", |w, _| w.fetches.last() == Some(&false)).await;
        let w = h.world.lock().unwrap();
        let after_ready: Vec<AccountState> = w.published.iter().map(|v| v.state).skip_while(|s| *s != AccountState::Ready).collect();
        assert_eq!(after_ready.iter().filter(|s| **s == AccountState::SignedOut).count(), 1, "{after_ready:?}");
    }

    /// Plan review C1: an account switch publishes Signed out inside the transition, then the next account's views.
    #[tokio::test]
    async fn a_switch_publishes_signed_out_and_then_the_next_account() {
        let h = ready_with(document("account.trialing.desktop.json")).await;
        h.until("Ready", |_, v| v.state == AccountState::Ready).await;
        h.world.lock().unwrap().facts = Facts { generation: 2, ..Facts::default() };
        h.handle.clear_for_sign_out();
        let cleared = h.world.lock().unwrap().published.last().cloned().expect("a published view");
        assert_eq!(cleared.state, AccountState::SignedOut, "inside the transition");
        h.respond(document("account.active.web.json"));
        h.with(|w| w.facts = Facts { identity: Identity::new(Some("u-2"), Some("kim@beebeeb.io")), ..signed_in(3) });
        h.until("Ready for the next account", |w, v| v.state == AccountState::Ready && w.fetches.len() >= 2).await;
        let w = h.world.lock().unwrap();
        let after_ready: Vec<AccountState> = w.published.iter().map(|v| v.state).skip_while(|s| *s != AccountState::Ready).collect();
        let out = after_ready.iter().position(|s| *s == AccountState::SignedOut).expect("Signed out was published after Ready");
        assert!(after_ready[out + 1..].contains(&AccountState::Ready), "the next account's view follows: {after_ready:?}");
    }

    /// Plan review I1: the driver sets the gate for the generation it derived. A generation it has not derived for reads
    /// closed; once it has, Ready under the new generation fires the account trigger (its own engine start was held).
    #[tokio::test]
    async fn the_driver_sets_the_gate_for_its_own_generation() {
        let h = ready_with(document("account.active.web.json")).await;
        h.until("Ready", |w, v| v.state == AccountState::Ready && w.ready == 1).await;
        assert_eq!(h.gate.current().generation, Some(1));
        assert!(h.gate.current().open_for(1));
        assert!(!h.gate.current().open_for(2), "a generation the driver has not derived for is closed");
        h.with(|w| w.facts.generation = 2);
        h.until("the gate follows the new generation", |_, _| h.gate.current().generation == Some(2)).await;
        assert!(h.gate.current().open_for(2));
        h.until("Ready under the new generation fires the trigger", |w, _| w.ready == 2).await;
    }

    /// Plan review I5 (lead ruling 2026-10-07): the restore task died and never reports. Checking still ends 15 s after
    /// the launch, as "document unavailable", and fails open; the fetches start.
    #[tokio::test]
    async fn a_dead_restore_task_still_ends_checking_after_15s() {
        let h = Harness::new(World { facts: signed_in(1), ..World::default() });
        h.handle.send(Event::Launch { restoring: true });
        h.until("Checking during the restore", |_, v| v.state == AccountState::Checking).await;
        h.clock.advance(Duration::from_secs(14));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!((h.handle.state().state, h.fetches()), (AccountState::Checking, 0), "before the watchdog");
        h.respond(document("account.active.web.json"));
        h.clock.advance(Duration::from_secs(1));
        h.until("left Checking and fetched with the bearer", |w, v| v.state == AccountState::Ready && w.fetches == [true]).await;
        assert!(h.gate.current().open_for(1));
    }

    /// Plan review I5: a sign-in while the restore is stuck gets its own 15 s. It always leaves Checking, and the
    /// launch's limit does not end it before its own first fetch could conclude.
    #[tokio::test]
    async fn a_sign_in_during_a_stuck_restore_leaves_checking() {
        let h = Harness::new(World { facts: signed_in(1), ..World::default() });
        h.handle.send(Event::Launch { restoring: true });
        h.until("Checking during the restore", |_, v| v.state == AccountState::Checking).await;
        h.clock.advance(Duration::from_secs(10));
        h.with(|w| w.facts = Facts { identity: Identity::new(Some("u-2"), Some("kim@beebeeb.io")), ..signed_in(2) });
        tokio::time::sleep(Duration::from_millis(50)).await;
        h.clock.advance(Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.handle.state().state, AccountState::Checking, "the launch's 15 s do not end the new session's Checking");
        assert!(!h.gate.current().open_for(2));
        h.clock.advance(Duration::from_secs(10));
        h.until("left Checking 15 s after the sign-in", |w, v| v.state == AccountState::Ready && w.fetches == [true]).await;
    }

    /// Spec §14 Binding: the cached row is kept after a session ended; only the in-memory copy goes.
    #[tokio::test]
    async fn the_cache_row_is_kept_after_a_session_ended() {
        let h = ready_with(document("account.needs_plan.ios.json")).await;
        h.until("No plan, cached", |w, v| v.state == AccountState::NoPlan && w.cache.is_some()).await;
        h.with(|w| w.facts.auth_expired = true);
        h.until("Session ended", |_, v| v.state == AccountState::SessionEnded).await;
        h.with(|w| w.facts = Facts { owner_recorded: true, generation: 2, ..Facts::default() });
        h.until("the relaunch's anonymous fetch", |w, v| w.fetches.last() == Some(&false) && v.state == AccountState::SessionEnded).await;
        let w = h.world.lock().unwrap();
        assert_eq!(w.deletes, 0, "never deleted");
        assert_eq!(w.cache.as_ref().map(|(identity, _)| identity.clone()), Some(sam()));
    }

    /// §10: a 401 or a 429 keeps the last document; only a new document replaces it.
    #[tokio::test]
    async fn a_401_or_a_429_keeps_the_last_document() {
        let h = ready_with(document("account.trialing.desktop.json")).await;
        h.until("Ready with the trial notice", |_, v| v.notice.is_some()).await;
        let before = h.handle.state();
        let hold = Arc::new(Notify::new());
        h.world.lock().unwrap().hold_fetch = Some(hold.clone());
        for (n, outcome) in [(2, FetchOutcome::Unauthorized), (3, FetchOutcome::RateLimited { retry_after: None })] {
            h.respond(outcome);
            h.handle.send(Event::Retry);
            h.until("the fetch is in flight", |w, _| w.fetches.len() == n).await;
            hold.notify_one();
        }
        h.handle.send(Event::Retry);
        h.until("the next fetch is in flight: both answers were applied and derived", |w, _| w.fetches.len() == 4).await;
        assert_eq!(h.handle.state(), before, "the trial notice and links are still the last document's");
        hold.notify_one();
    }

    #[tokio::test]
    async fn settled_returns_once_checking_ends_or_at_the_deadline() {
        let (handle, _rx) = AccountViewHandle::for_test(AccountView { state: AccountState::Checking, ..AccountView::signed_out(false) });
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        assert_eq!(handle.settled(deadline).await.state, AccountState::Checking, "still Checking at the deadline");
        let waiter = {
            let handle = handle.clone();
            tokio::spawn(async move { handle.settled(tokio::time::Instant::now() + Duration::from_secs(5)).await })
        };
        handle.published.publish(AccountView::signed_out(false), || true);
        assert_eq!(waiter.await.unwrap().state, AccountState::SignedOut);
    }

    #[test]
    fn the_log_entries_map_to_the_closed_vocabulary() {
        assert_eq!(
            AccountLog::State { from: AccountState::Checking, to: AccountState::NoPlan, offline: false }.line(),
            "account_state from=checking to=no_plan offline=no"
        );
        assert_eq!(AccountLog::EngineHeld(HeldReason::UpdateRequired).line(), "engine_held reason=update_required");
        assert_eq!(AccountLog::Document(DocumentOutcome::BlockingUnknownStep).line(), "onboarding_document outcome=blocking_unknown_step");
        assert_eq!(
            AccountLog::EngineHeld(HeldReason::Checking).lifecycle_event(),
            LifecycleEvent::EngineHeld { reason: "checking" }
        );
    }
}
```

`a_launch_without_a_session_fetches_anonymously_and_shows_sign_in` relies on `World::default()`'s `Facts` (no token, no owner). `LifecycleEvent` derives `PartialEq` (spec A).

- [ ] **Step 4: Run them to see them fail, then pass**

First save (do not commit) the step 2 code with every method body replaced by `todo!()` except the type definitions, run with filter `account_view::driver`, and paste the failure summary (every async test panics with `not yet implemented`). Then restore the bodies from step 2 and run again.
Expected: `test result: ok. 34 passed; 0 failed`. Then the whole lib under the scratch-HOME recipe: on macOS `B_lib` + 135 (the running total in Global Constraints). If any `until` panics with "never:", read its view in the message before touching code: a stuck wait is first a question about the harness (did the test queue a response? did the clock move?).

- [ ] **Step 5: Mutation-check (spec §14: binding, trigger, schedule)**

1. Delete the `if after.generation != before.generation { return; }` block. Expected: `a_result_fetched_for_an_earlier_session_is_dropped_and_never_cached`. Revert.
2. In `apply`, change `entered_ready` to `state == AccountState::Ready`. Expected: `account_ready_fires_on_no_other_edge` and `account_ready_fires_at_launch_from_the_cache`. Revert.
3. In `load_cache`, return `Some(part)` for `Some(false)`. Expected: `another_accounts_cache_row_is_deleted_and_never_shown`. Revert.
4. In `fetch`, drop `if after.identity.user_id.is_some()` (always write). Expected: `an_unidentified_session_keeps_its_document_in_memory_until_its_identity_is_known`. Revert.
5. In `fetch`'s `Network` arm, also skip `self.first_fetch = FirstFetch::Concluded;` (move that line into the other arms only). Expected: `a_first_network_failure_ends_checking_without_showing_offline`. Revert.
6. In `apply`, replace `|| ports.generation() == generation` with `|| true`. Expected: `a_view_derived_for_an_ended_session_never_overwrites_the_cleared_one` ("the ended session's view was never published"). Revert.
7. In `apply`, log `EngineHeld` only when `closed_now`. Expected: `closing_the_gate_holds_once_and_says_why` (no `account_blocking` line: the gate was already closed for Checking). Revert.
8. In `clear_for_sign_out`, replace the `self.published.publish(…)` call with `self.published.view.send_replace(AccountView::signed_out(offline));` (the clear writes only the watch). Expected: `a_live_sign_out_publishes_signed_out` ("published inside the transition": the last published view is still Ready) and `a_switch_publishes_signed_out_and_then_the_next_account` ("inside the transition"). Revert.
9. In `apply`, pass `0` instead of `generation` to `self.gate.set`. Expected: `the_driver_sets_the_gate_for_its_own_generation` (left `Some(0)`). Revert.
10. Drop `|| opened_now` from `fire_ready`. Expected: `the_driver_sets_the_gate_for_its_own_generation` ("never: Ready under the new generation fires the trigger"). Revert.
11. Make `watchdog` return at once. Expected: `a_dead_restore_task_still_ends_checking_after_15s` and `a_sign_in_during_a_stuck_restore_leaves_checking` ("never: left Checking…"). Revert.
12. In `refresh`, delete the `self.checking_until = Some(…)` line of the generation change. Expected: `a_sign_in_during_a_stuck_restore_leaves_checking` ("the launch's 15 s do not end the new session's Checking"). Revert.
13. In `refresh`'s `else` branch (no token), add `ports.delete_cache();`. Expected: `the_cache_row_is_kept_after_a_session_ended` ("never deleted"). Revert.
14. In `fetch`'s `Unauthorized` arm, add `self.fresh = None;`. Expected: `a_401_or_a_429_keeps_the_last_document` (the right side has the trial notice, the left has none). Revert.
15. In `Event::RestoreFinished`, set `self.fetch_now = true;` unconditionally. Expected: `a_launch_without_a_session_fetches_anonymously_and_shows_sign_in` ("the restore's end after a signed-out launch fetches nothing more"). Revert.

- [ ] **Step 6: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t10-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/driver.rs src-tauri/src/account_view/mod.rs src-tauri/src/lifecycle_log.rs
git commit -m "desktop: one account-view task that fetches, caches, derives and holds" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/driver.rs src-tauri/src/account_view/mod.rs src-tauri/src/lifecycle_log.rs
git show --stat HEAD
```

---

## Task 11: Wire the account view into the app — ports, launch, commands and session events

**Lane R.** The production `Ports` and `Publisher`, arming the gate, the four commands, and one event from every place the session changes.

**Before you start:** read `$EVID/t0-record.md` (the session generation's name, and whether `SESSION_TRANSITION` and `start_engine_if_possible`'s `_transition` parameter are Windows-only).

**Files:**
- Create: `src-tauri/src/account_view/app_ports.rs`
- Modify: `src-tauri/src/account_view/mod.rs` (`pub mod app_ports;`)
- Modify: `src-tauri/src/lib.rs` (drop the `#[allow(dead_code)]` on `mod account_view;` and on `hold_for_account`; `AppState.account_view`; `account_view_facts`, `account_view_token`, `notify_account_view`, `start_engine_when_none_runs`, `RestoreFinishedGuard`; `setup()`; `keys_arrived`; `clear_session_impl`; `lock_vault`; `desktop_login`; `desktop_login_2fa`; the focus handler; `attach_tray_status_listener`; four commands; tests)

**Interfaces:**
- Consumes: `driver::{spawn, AccountViewHandle, Event, Facts, CacheRead, Ports, Publisher, AccountLog, ACCOUNT_VIEW_CHANGED_EVENT}` (Task 10), `derive::{derive, Inputs, TokenState, FirstFetch, AccountView}` (Task 5), `fetch::{client, fetch, FetchOutcome, UnavailableKind}` (Task 8), `gate::AccountGate` and `hold_for_account` (Task 9), `StateDb::{onboarding_cache, set_onboarding_cache, delete_onboarding_cache, owner}` (Task 7); spec A's `identity_of_session`, `keychain_vault_key_present`, `load_session_token_from_keychain`, `state_db_from_state_dir`, `state_paths::{beebeeb_state_dir, STATE_DB_FILENAME}`, `runner::api_base_url`, `AuthHealth::note_result`, `notify_finder`, `start_engine_if_possible`, `lifecycle_log::event`, `finder_setup::driver::SystemClock`, and Task 12-of-spec-A's `session_generation()` (its real name from Task 0).
- Produces:
  - `AppState.account_view: std::sync::OnceLock<account_view::driver::AccountViewHandle>`
  - `account_view::app_ports::AppPorts::new(app: tauri::AppHandle)`, `account_view::app_ports::AppPublisher::new(app: tauri::AppHandle)` (`impl Publisher`: emits `account-view-changed`; Task 14 adds the menu-bar icon)
  - `lib.rs`: `struct RestoreFinishedGuard(tauri::AppHandle)` (sends `RestoreFinished` when dropped)
  - `lib.rs`: `pub(crate) fn account_view_facts(state: &AppState) -> account_view::driver::Facts`, `fn account_view_token(state: &AppState) -> Option<zeroize::Zeroizing<String>>`, `fn notify_account_view(state: &AppState, event: account_view::driver::Event)`, `async fn start_engine_when_none_runs(app: tauri::AppHandle, state: &State<'_, AppState>)` (not macOS)
  - Tauri commands: `account_view_state() -> Result<AccountView, String>`, `account_view_retry() -> Result<(), String>`, `account_view_plan_opened() -> Result<(), String>`, `quit_app()`

- [ ] **Step 1: Write the failing tests**

In `finder_setup_command_tests` (it has `production_source` and `body_between`):

```rust
    /// Spec 2026-10-07: the account view hears about every session transition.
    #[test]
    fn the_account_view_is_told_about_every_session_transition() {
        let production = production_source();
        for (function, event) in [
            ("fn keys_arrived(", "Event::SessionChanged"),
            ("async fn lock_vault(", "Event::SessionChanged"),
        ] {
            let body = body_between(&production, function, "\n}\n");
            assert!(body.contains("notify_account_view(") && body.contains(event), "{function}");
        }
        for login in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let body = body_between(&production, login, "\n}\n");
            let outcomes = body.matches("Ok(LoginOutcome").count();
            assert!(outcomes > 0, "{login}");
            assert_eq!(body.matches("notify_account_view(").count(), outcomes, "{login}: every returned outcome tells the account view");
        }
    }

    /// C-D5: the sign-out clears the account view inside its own transition, after it has finished.
    #[test]
    fn a_sign_out_clears_the_account_view_inside_its_transition() {
        let production = production_source();
        let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
        let signed_out = clear.find("LifecycleEvent::SignedOut").expect("the sign-out's log line");
        let cleared = clear.find("clear_for_sign_out()").expect("the account view is cleared");
        assert!(signed_out < cleared, "cleared once the sign-out is done");
    }

    /// C-R10 and plan review I1: the gate is armed before the restore can start an engine, so every generation the
    /// account view has not derived for reads closed from the first moment.
    #[test]
    fn the_launch_gate_is_set_before_the_restore_can_start_an_engine() {
        let production = production_source();
        let gate = production.find("state.account_gate.arm(").expect("the gate is armed in setup");
        let restore = production.find("restore_session_on_startup(&h).await").expect("the restore");
        assert!(gate < restore);
    }

    /// Plan review I5: the startup restore ends with `RestoreFinished` even when it panics or its task is aborted. A
    /// drop guard is created before the restore and dropped right after it.
    #[test]
    fn the_restore_always_ends_with_restore_finished() {
        let production = production_source();
        let guard = production.find("let restore_finished = RestoreFinishedGuard(").expect("the guard");
        let restore = production.find("restore_session_on_startup(&h).await").expect("the restore");
        let dropped = production.find("drop(restore_finished);").expect("dropped after the restore");
        assert!(guard < restore && restore < dropped);
        let on_drop = body_between(&production, "impl Drop for RestoreFinishedGuard", "\n}\n");
        assert!(on_drop.contains("Event::RestoreFinished"), "{on_drop}");
    }

    /// Review Focus 3, plan "Spec issues" 2: on Windows and Linux the account trigger starts the engine only when none
    /// runs, so a session that recovers by itself never restarts sync.
    #[test]
    fn account_ready_starts_the_engine_only_when_none_runs() {
        let production = production_source();
        let body = body_between(&production, "async fn start_engine_when_none_runs(", "\n}\n");
        let running = body.find("engine.lock().await.is_some()").expect("the slot is read first");
        let start = body.find("start_engine_if_possible(").expect("today's start");
        assert!(running < start);
        let ports = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/account_view/app_ports.rs")).unwrap();
        let ready = &ports[ports.find("fn account_ready(").expect("account_ready")..];
        assert!(ready.contains("Trigger::AccountReady") && ready.contains("start_engine_when_none_runs("));
        assert!(!ready.contains("start_engine_if_possible("), "never today's start directly");
    }

    #[test]
    fn the_account_view_commands_are_registered() {
        let production = production_source();
        let handler = &production[production.find("tauri::generate_handler![").expect("the handler list")..];
        for command in ["account_view_state", "account_view_retry", "account_view_plan_opened", "quit_app"] {
            assert!(handler.contains(command), "{command}");
        }
    }
```

In the R10 test module (beside Task 9's `a_closed_account_gate_starts_nothing_and_binds_nothing`):

```rust
    /// Review Focus 1, plan "Spec issues" 3: what the gate reads while Locked with a cached no-plan part is what the
    /// unlock's engine start meets, before the account view has re-derived anything.
    #[test]
    fn an_unlock_with_a_cached_no_plan_part_starts_no_engine() {
        use crate::account_view::derive::{derive, AccountState, FirstFetch, Inputs, TokenState};
        let part = crate::account_view::doc::tests::account_part("account.needs_plan.ios.json");
        let locked = derive(&Inputs {
            platform: crate::surfaces::policy::Platform::current(),
            token: TokenState::Stored,
            auth_expired: false,
            owner_recorded: true,
            key_in_memory: false,
            key_in_keychain: true,
            fresh: None,
            cached: Some(&part),
            first_fetch: FirstFetch::Concluded,
            offline: false,
        });
        assert_eq!(locked.view.state, AccountState::Locked);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let local = local_data_of(Some(bob()));
            let fx = AuthorizeFixture::with_session(&bob());
            let platform = crate::surfaces::policy::Platform::current();
            fx.state.account_gate.arm(platform != crate::surfaces::policy::Platform::Linux);
            fx.state.account_gate.set(locked.gate_open, crate::session_generation());
            let mut slot = fx.acct.engine.lock().await;
            let paths = LocalDataPaths::for_test(local.dir.path(), &local.staging);
            let result = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                crate::runner::EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            if cfg!(target_os = "linux") {
                assert_ne!(result, Ok(EngineStart::AccountBlocked), "Linux never holds (C-R14)");
            } else {
                assert_eq!(result, Ok(EngineStart::AccountBlocked));
                assert!(slot.is_none());
            }
        });
    }
```

Run with the filter `-- the_account_view_is_told_about_every_session_transition a_sign_out_clears_the_account_view_inside_its_transition the_launch_gate_is_set_before_the_restore_can_start_an_engine the_restore_always_ends_with_restore_finished account_ready_starts_the_engine_only_when_none_runs the_account_view_commands_are_registered an_unlock_with_a_cached_no_plan_part_starts_no_engine`. Expected: `7` tests run; the six source tests fail on their `expect` (nothing is wired yet); `an_unlock_with_a_cached_no_plan_part_starts_no_engine` already passes (Tasks 5 and 9 did the work) — note that in the Notes, and mutation-check it in step 6 instead.

- [ ] **Step 2: The production ports**

`src-tauri/src/account_view/app_ports.rs`:

```rust
//! Spec 2026-10-07: the account-view driver's ports in the running app. Every read comes from the same sources spec A
//! uses (the session in memory, `auth_present`, `AuthHealth`, the owner record, the Keychain presence check), and every
//! action goes through spec A's own helpers (`hold_for_account`, `notify_finder`, today's engine start).

use std::future::Future;

use tauri::{Emitter, Manager};

use super::derive::AccountView;
use super::doc::AccountPart;
use super::driver::{AccountLog, CacheRead, Facts, Ports, Publisher, ACCOUNT_VIEW_CHANGED_EVENT};
use super::fetch::{self, FetchOutcome, UnavailableKind};
use crate::account_binding::Identity;
use crate::surfaces::policy::Platform;
use crate::AppState;

pub struct AppPorts {
    app: tauri::AppHandle,
    client: Option<reqwest::Client>,
}

impl AppPorts {
    pub fn new(app: tauri::AppHandle) -> Self {
        let client = match fetch::client(Platform::current()) {
            Ok(client) => Some(client),
            Err(error) => {
                tracing::warn!(%error, "account view: no HTTP client; every fetch counts as a network failure");
                None
            }
        };
        Self { app, client }
    }
}

/// Plan review C1: where every published view goes besides the account view's watch channel. It is called under the
/// publish lock, so it never blocks: the event is queued for the windows, and the menu-bar icon (Task 14) is posted to
/// the main thread, in publish order.
pub struct AppPublisher {
    app: tauri::AppHandle,
}

impl AppPublisher {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Publisher for AppPublisher {
    fn publish(&self, view: &AccountView) {
        if let Err(error) = self.app.emit(ACCOUNT_VIEW_CHANGED_EVENT, view) {
            tracing::warn!(%error, "account view: could not emit the changed event");
        }
    }
}

/// `state.db`, created when it does not exist yet (a first sign-in caches before the first engine start). The cache
/// row is not account data (plan "Spec issues" 9), so creating the database early changes nothing R10 decides.
fn open_or_create_state_db() -> Result<crate::state_db::StateDb, String> {
    let dir = crate::state_paths::beebeeb_state_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create the state directory: {e}"))?;
    crate::state_db::StateDb::open(dir.join(crate::state_paths::STATE_DB_FILENAME)).map_err(|e| format!("open state.db: {e}"))
}

fn existing_state_db() -> Result<Option<crate::state_db::StateDb>, String> {
    crate::state_db_from_state_dir(&crate::state_paths::beebeeb_state_dir()?)
}

impl Ports for AppPorts {
    fn platform(&self) -> Platform {
        Platform::current()
    }

    fn facts(&self) -> Facts {
        crate::account_view_facts(&self.app.state::<AppState>())
    }

    fn generation(&self) -> u64 {
        crate::session_generation()
    }

    fn fetch(&mut self, bearer: bool) -> impl Future<Output = FetchOutcome> + Send {
        let client = self.client.clone();
        let token = if bearer { crate::account_view_token(&self.app.state::<AppState>()) } else { None };
        async move {
            let Some(client) = client else {
                return FetchOutcome::Network;
            };
            // A signed-in fetch whose token cannot be read is "unavailable", never an anonymous fetch.
            if bearer && token.is_none() {
                return FetchOutcome::Unavailable(UnavailableKind::Unreadable);
            }
            fetch::fetch(&client, &crate::runner::api_base_url(), token.as_ref().map(|token| token.as_str())).await
        }
    }

    fn read_cache(&mut self) -> CacheRead {
        let row = match existing_state_db() {
            Ok(Some(db)) => db.onboarding_cache(),
            Ok(None) => return CacheRead::Empty,
            Err(_) => return CacheRead::Unreadable,
        };
        match row {
            Ok(Some(row)) => match serde_json::from_str::<AccountPart>(&row.account_part) {
                Ok(part) => CacheRead::Row { identity: row.identity, part },
                Err(_) => CacheRead::Unreadable,
            },
            Ok(None) => CacheRead::Empty,
            Err(_) => CacheRead::Unreadable,
        }
    }

    fn write_cache(&mut self, identity: &Identity, part: &AccountPart) {
        let result = serde_json::to_string(part)
            .map_err(|e| format!("encode the account part: {e}"))
            .and_then(|json| {
                let at = chrono::Utc::now().timestamp();
                open_or_create_state_db()?.set_onboarding_cache(identity, at, &json).map_err(|e| format!("write the cache row: {e}"))
            });
        if let Err(error) = result {
            tracing::warn!(%error, "account view: the account part was not cached");
        }
    }

    fn delete_cache(&mut self) {
        if let Ok(Some(db)) = existing_state_db()
            && let Err(error) = db.delete_onboarding_cache()
        {
            tracing::warn!(%error, "account view: could not delete another account's cache row");
        }
    }

    fn note_auth(&mut self, unauthorized: bool) {
        let state = self.app.state::<AppState>();
        if let Ok(acct) = state.active_account() {
            if unauthorized {
                acct.auth_health.note_result(Some(&anyhow::anyhow!("HTTP 401 Unauthorized")));
            } else {
                acct.auth_health.note_result(None);
            }
        }
    }

    fn log(&mut self, entry: AccountLog) {
        tracing::info!(target: "account_view", "{}", entry.line());
        #[cfg(target_os = "macos")]
        crate::lifecycle_log::event(entry.lifecycle_event());
    }

    fn gate_closed(&mut self) -> impl Future<Output = ()> + Send {
        let app = self.app.clone();
        async move {
            let state = app.state::<AppState>();
            crate::hold_for_account(&state).await;
        }
    }

    fn account_ready(&mut self) -> impl Future<Output = ()> + Send {
        let app = self.app.clone();
        async move {
            let state = app.state::<AppState>();
            // C-R12, macOS: like spec A's KeysArrived, it lifts the reconciler's hold and asks for a check.
            #[cfg(target_os = "macos")]
            crate::notify_finder(&state, crate::finder_setup::core::Trigger::AccountReady);
            // C-R12, Windows and Linux: today's engine start, when no engine runs (plan "Spec issues" 2).
            #[cfg(not(target_os = "macos"))]
            crate::start_engine_when_none_runs(app.clone(), &state).await;
        }
    }
}
```

If `state_db_from_state_dir` is private, make it `pub(crate)`; same for `hold_for_account`, `notify_finder`, `account_view_facts`, `account_view_token`, `start_engine_when_none_runs`.

- [ ] **Step 3: The facts, the token and the helpers in `lib.rs`**

`AppState`, after `account_gate`:

```rust
    /// Spec 2026-10-07: the account view (one task for the app's life, set in `setup()`).
    pub account_view: std::sync::OnceLock<account_view::driver::AccountViewHandle>,
```

and in `Default`: `account_view: std::sync::OnceLock::new(),`. Remove the `#[allow(dead_code)]` and its comment from `mod account_view;`.

Beside `keys_arrived`:

```rust
/// Spec 2026-10-07 §4.1: what the account view derives from, read from the app's real sources. An owner record that
/// cannot be read counts as recorded (fail closed: the person is asked to sign in again, never treated as new).
///
/// The account view refreshes on every event, tick and fetch, so the two slow reads (`state.db` and the Keychain) run
/// only where the table reads them (plan review M7): the owner record decides only without a stored token (C-R1 or
/// C-R2), and the Keychain only for a stored token whose key is not in memory (which C-R4 screen). Elsewhere they are
/// `false` without a read. No cache: a cached owner record could outlive the sign-out's purge.
pub(crate) fn account_view_facts(state: &AppState) -> account_view::driver::Facts {
    let acct = state.active_account().ok();
    let (key_in_memory, session_email) = acct
        .as_ref()
        .and_then(|acct| acct.session.lock().ok().map(|guard| (guard.is_some(), guard.as_ref().and_then(|s| s.email.clone()))))
        .unwrap_or((false, None));
    let email = session_email.or_else(|| acct.as_ref().and_then(|acct| acct.auth_email.lock().ok().and_then(|guard| guard.clone())));
    let profile = acct.as_ref().and_then(|acct| acct.cached_profile.lock().ok().and_then(|guard| guard.clone()));
    let token_stored = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    let owner_recorded = !token_stored
        && match state_paths::beebeeb_state_dir().and_then(|dir| state_db_from_state_dir(&dir)) {
            Ok(Some(db)) => db.owner().map(|owner| owner.is_some()).unwrap_or(true),
            Ok(None) => false,
            Err(_) => true,
        };
    let key_in_keychain = token_stored && !key_in_memory && acct.as_ref().is_some_and(|acct| vault_key_in_keychain(acct.id.as_str()));
    account_view::driver::Facts {
        token_stored,
        auth_expired: acct.as_ref().is_some_and(|acct| acct.auth_health.is_expired()),
        owner_recorded,
        key_in_memory,
        key_in_keychain,
        identity: identity_of_session(email.as_deref(), profile.as_ref()),
        generation: session_generation(),
    }
}

#[cfg(not(target_os = "windows"))]
fn vault_key_in_keychain(account_id: &str) -> bool {
    keychain_vault_key_present(account_id)
}

/// Windows has no Keychain unlock screen to route to (C-W7 is macOS).
#[cfg(target_os = "windows")]
fn vault_key_in_keychain(_account_id: &str) -> bool {
    false
}

/// The session token for a signed-in fetch: the one in memory, else the stored one (a locked vault keeps its token).
pub(crate) fn account_view_token(state: &AppState) -> Option<zeroize::Zeroizing<String>> {
    let acct = state.active_account().ok()?;
    if let Some(token) = acct.session.lock().ok()?.as_ref().map(|session| zeroize::Zeroizing::new(session.token.clone())) {
        return Some(token);
    }
    load_session_token_from_keychain(acct.id.as_str()).ok().flatten().map(zeroize::Zeroizing::new)
}

/// Tell the account view something happened. A no-op before `setup()` has started it (and in unit tests).
pub(crate) fn notify_account_view(state: &AppState, event: account_view::driver::Event) {
    if let Some(handle) = state.account_view.get() {
        handle.send(event);
    }
}

/// Spec 2026-10-07 C-R12 on Windows and Linux: today's engine start, but only when no engine runs, so a transition
/// into Ready never restarts sync that is already going (plan "Spec issues" 2).
#[cfg(not(target_os = "macos"))]
pub(crate) async fn start_engine_when_none_runs(app: tauri::AppHandle, state: &State<'_, AppState>) {
    #[cfg(target_os = "windows")]
    let transition = SESSION_TRANSITION.lock().await;
    let running = match state.active_account() {
        Ok(acct) => acct.engine.lock().await.is_some(),
        Err(_) => return,
    };
    if running {
        return;
    }
    if let Err(error) = start_engine_if_possible(
        app,
        state,
        #[cfg(target_os = "windows")]
        &transition,
    )
    .await
    {
        tracing::warn!(%error, "account ready: the sync engine did not start");
    }
}
```

`identity_of_session`'s first parameter is the session email; use the signature `t0-record.md` records. Per `t0-record.md`: if `SESSION_TRANSITION` is on every platform and `start_engine_if_possible`'s `_transition` parameter is no longer Windows-only, take the lock on every platform and pass it unconditionally (drop both `#[cfg(target_os = "windows")]` lines in `start_engine_when_none_runs`); if both are still Windows-only, keep them as shown. On Linux the function must compile with the argument list `start_engine_if_possible` has there.

Remove the `#[allow(dead_code)]` line and its comment above `hold_for_account` (Task 9): `AppPorts::gate_closed` calls it now.

Beside `notify_account_view`:

```rust
/// Plan review I5: tells the account view that the startup restore is over when it is dropped, so also when the
/// restore panics or its task is aborted. The account view never waits for a restore that cannot finish.
struct RestoreFinishedGuard(tauri::AppHandle);

impl Drop for RestoreFinishedGuard {
    fn drop(&mut self) {
        if let Some(state) = self.0.try_state::<AppState>() {
            notify_account_view(&state, account_view::driver::Event::RestoreFinished);
        }
    }
}
```

- [ ] **Step 4: Launch, restore, session events, focus, auth-expired, commands**

In `setup()`, directly after the block that seeds `auth_present` (`*guard = keychain_session_present(&account_id);`) and before the Finder reconciler block:

```rust
            // Spec 2026-10-07: the account view. Its gate is armed before the restore below can start an engine: armed,
            // it reads closed for every session generation the account view has not derived for (C-R11, plan review
            // I1). Linux arms it so that it never holds (C-R10). The launch view is the first view published.
            {
                use account_view::derive::{FirstFetch, Inputs, TokenState};
                let state = app.state::<AppState>();
                let facts = account_view_facts(&state);
                let restoring = facts.token_stored;
                let launch = account_view::derive::derive(&Inputs {
                    platform: surfaces::policy::Platform::current(),
                    token: if restoring { TokenState::Restoring } else { TokenState::Absent },
                    auth_expired: facts.auth_expired,
                    owner_recorded: facts.owner_recorded,
                    key_in_memory: false,
                    key_in_keychain: facts.key_in_keychain,
                    fresh: None,
                    cached: None,
                    first_fetch: FirstFetch::Pending,
                    offline: false,
                });
                state.account_gate.arm(surfaces::policy::Platform::current() != surfaces::policy::Platform::Linux);
                let handle = account_view::driver::spawn(
                    account_view::app_ports::AppPorts::new(app.handle().clone()),
                    finder_setup::driver::SystemClock,
                    state.account_gate.clone(),
                    std::sync::Arc::new(account_view::app_ports::AppPublisher::new(app.handle().clone())),
                    launch.view,
                );
                handle.send(account_view::driver::Event::Launch { restoring });
                let _ = state.account_view.set(handle);
            }
```

In the restore task, around `restore_session_on_startup(&h).await;`:

```rust
                    // Plan review I5: the account view hears `RestoreFinished` even if the restore panics or is aborted.
                    let restore_finished = RestoreFinishedGuard(h.clone());
                    restore_session_on_startup(&h).await;
                    drop(restore_finished);
```

Spec A's `the_launch_trigger_follows_the_startup_restore` allows at most 400 characters from `restore_session_on_startup(&h).await` to the Finder `Launch` trigger (267 at spec A's HEAD). Only the `drop(restore_finished);` line goes between them; the comment and the guard sit above the restore call. Measure the new distance and paste it in the Notes.

`keys_arrived`: after the `notify_finder(…KeysArrived)` line:

```rust
    notify_account_view(state, account_view::driver::Event::SessionChanged);
```

`lock_vault`: as its last statement before returning `Ok(())`, `notify_account_view(&state, account_view::driver::Event::SessionChanged);`.

`desktop_login` and `desktop_login_2fa`: immediately before every `Ok(LoginOutcome…)` they return (`grep -n 'Ok(LoginOutcome' src-tauri/src/lib.rs`), add `notify_account_view(&state, account_view::driver::Event::SessionChanged);` (a password sign-in without the key reaches Locked; with the key in place, Checking).

`clear_session_impl`: directly after `lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedOut);`:

```rust
    // Spec 2026-10-07 C-D5: the leaving account's document, notice and links are gone before this returns.
    if let Some(handle) = state.account_view.get() {
        handle.clear_for_sign_out();
    }
```

The focus handler (`.on_window_event`), inside the existing `Focused(true)` block or beside it:

```rust
            if let tauri::WindowEvent::Focused(true) = event {
                notify_account_view(&window.state::<AppState>(), account_view::driver::Event::WindowFocused);
            }
```

`attach_tray_status_listener`, after the block that mirrors `engine_state`: tell the account view when spec A's auth-expired state flips (once per change, never per engine tick):

```rust
        // Spec 2026-10-07 C-R2 (a): the auth-expired state flips without a session event; the account view re-derives
        // once per change.
        if let Some(app_state) = app_for_listener.try_state::<AppState>()
            && let Ok(acct) = app_state.active_account()
        {
            let expired = if acct.auth_health.is_expired() { 2 } else { 1 };
            if LAST_AUTH_EXPIRED.swap(expired, std::sync::atomic::Ordering::SeqCst) != expired {
                notify_account_view(&app_state, account_view::driver::Event::SessionChanged);
            }
        }
```

with, at module level: `static LAST_AUTH_EXPIRED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);`.

The commands, beside `open_reauth_window`:

```rust
/// Spec 2026-10-07 §13: the account view every window renders.
#[tauri::command]
fn account_view_state(state: State<'_, AppState>) -> Result<account_view::derive::AccountView, String> {
    state.account_view.get().map(|handle| handle.state()).ok_or_else(|| "The account state isn’t available yet.".to_string())
}

/// "Try again" (§7.1, §7.3, §7.4): fetch the onboarding document now.
#[tauri::command]
fn account_view_retry(state: State<'_, AppState>) -> Result<(), String> {
    let handle = state.account_view.get().ok_or_else(|| "The account state isn’t available yet.".to_string())?;
    handle.send(account_view::driver::Event::Retry);
    Ok(())
}

/// "Choose a plan on beebeeb.io" was clicked (§7.4): re-check every `poll_seconds` for 15 minutes.
#[tauri::command]
fn account_view_plan_opened(state: State<'_, AppState>) -> Result<(), String> {
    let handle = state.account_view.get().ok_or_else(|| "The account state isn’t available yet.".to_string())?;
    handle.send(account_view::driver::Event::PlanOpened);
    Ok(())
}

/// "Quit Beebeeb" in the sign-in window (§7.1).
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}
```

and add `account_view_state, account_view_retry, account_view_plan_opened, quit_app,` to `tauri::generate_handler![…]`.

- [ ] **Step 5: Run the tests to see them pass**

Run the step 1 filter, then the whole lib under the scratch-HOME recipe, then `cargo test --locked` for every binary.
Expected: `test result: ok. 7 passed; 0 failed` for the filter; on macOS the lib is `B_lib` + 142 (the running total in Global Constraints); every other binary has Task 0's baseline count; `0 failed` everywhere. `the_launch_trigger_follows_the_startup_restore` (spec A) passes.

- [ ] **Step 6: Mutation-check**

1. Remove `notify_account_view(…SessionChanged)` from `keys_arrived`. Expected: `the_account_view_is_told_about_every_session_transition`. Revert.
2. Move `handle.clear_for_sign_out()` above the `SignedOut` log line. Expected: `a_sign_out_clears_the_account_view_inside_its_transition`. Revert.
3. In `start_engine_when_none_runs`, delete the `if running { return; }`. Expected: no test fails on it alone, because the source test reads the order of the two calls; replace `engine.lock().await.is_some()` with `false`, then expect `account_ready_starts_the_engine_only_when_none_runs`. Revert.
4. In `derive.rs`, compute `gate_open` from `state` (Task 5's mutation 4) and run `an_unlock_with_a_cached_no_plan_part_starts_no_engine`. Expected: it fails on macOS and Windows. Revert.
5. Delete the `drop(restore_finished);` line. Expected: `the_restore_always_ends_with_restore_finished` ("dropped after the restore"). Revert. Then delete the `notify_account_view(…RestoreFinished)` call inside `impl Drop`. Expected: the same test (the `on_drop` assertion). Revert.
6. Delete the `state.account_gate.arm(…)` line in `setup()`. Expected: `the_launch_gate_is_set_before_the_restore_can_start_an_engine`. Revert.

- [ ] **Step 7: Commit**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t11-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/account_view/app_ports.rs src-tauri/src/account_view/mod.rs src-tauri/src/lib.rs
git commit -m "desktop: run the account view in the app and tell it about every session change" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/account_view/app_ports.rs src-tauri/src/account_view/mod.rs src-tauri/src/lib.rs
git show --stat HEAD
```

---

## Task 12: One sign-in outcome for both paths (spec C-S1) and the switch count carry-over (T11-M5)

**Lane R.** `start_browser_login` returns `LoginOutcome`, the same shape `desktop_login` does; a different account gets the switch warning through it; nothing raw reaches the frontend. Steps 6–8 (T11-M5) run only if Task 0 found it open.

**Files:**
- Modify: `src-tauri/src/browser_login.rs` (`start_browser_login`, `run_handoff`)
- Modify: `src-tauri/src/lib.rs` (`LoginOutcome::browser_signed_in`, `browser_settlement`, `browser_installed_outcome`; M5: `pending_changes_count`, `AccountMismatchDto`, `SignInSettlement::AccountMismatch`, `LoginOutcome::account_mismatch`, `gather_local_facts`)

**Interfaces:**
- Consumes: spec A's `LoginOutcome::{signed_in, reauthenticated, account_mismatch}`, `SignInSettlement::{Fresh, Reauthenticated { vault_unlocked }, AccountMismatch { pending_changes }, Unconfirmed}`, `SIGN_IN_ACCOUNT_UNKNOWN`, `revoke_desktop_session`, `apply_session`, `settle_sign_in`.
- Produces:
  - `#[tauri::command] start_browser_login(…) -> Result<LoginOutcome, String>` — the JSON the frontend parses with `settledFrom` (Task 16):
    - fresh: `{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":true,"account_mismatch":null}`
    - same account: `{"requires_2fa":false,"reauthenticated":true,"vault_unlocked":true,"account_mismatch":null}`
    - another account: `{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":N}}` (`N` is `null` after step 8 when the count cannot be read)
    - `Err(SIGN_IN_ACCOUNT_UNKNOWN)` stays the retryable "couldn't confirm" sentence (ruling T11f1-c2)
  - `LoginOutcome::browser_signed_in() -> LoginOutcome`
  - `enum BrowserSettlement { Return(Result<LoginOutcome, String>), RevokeAndReturn(Result<LoginOutcome, String>), Install { reauthenticated: bool } }`, `fn browser_settlement(Option<SignInSettlement>) -> BrowserSettlement`, `fn browser_installed_outcome(reauthenticated: bool) -> LoginOutcome` (every platform: Windows calls it at the end of `run_handoff` too)

- [ ] **Step 1: Write the failing tests**

In `lib.rs`'s R8 test module (the one that holds `login_outcome_json_is_the_frontends_contract`):

```rust
    /// C-S1: a fresh browser sign-in delivers the keys, so its outcome says so.
    #[test]
    fn a_browser_sign_in_reports_the_vault_unlocked() {
        assert_eq!(
            serde_json::to_value(browser_installed_outcome(false)).unwrap(),
            serde_json::json!({ "requires_2fa": false, "reauthenticated": false, "vault_unlocked": true, "account_mismatch": null })
        );
        assert_eq!(
            serde_json::to_value(browser_installed_outcome(true)).unwrap(),
            serde_json::json!({ "requires_2fa": false, "reauthenticated": true, "vault_unlocked": true, "account_mismatch": null })
        );
    }

    /// C-S1: every settlement maps to the password path's outcome; another account is the switch outcome, not an error.
    #[test]
    fn a_browser_sign_in_settles_like_the_password_path() {
        let json = |result: &Result<LoginOutcome, String>| result.as_ref().map(|outcome| serde_json::to_value(outcome).unwrap()).map_err(Clone::clone);
        match browser_settlement(Some(SignInSettlement::AccountMismatch { pending_changes: 3 })) {
            BrowserSettlement::RevokeAndReturn(result) => assert_eq!(
                json(&result),
                Ok(serde_json::json!({ "requires_2fa": false, "reauthenticated": false, "vault_unlocked": false, "account_mismatch": { "pending_changes": 3 } }))
            ),
            _ => panic!("another account revokes the new session and returns the switch outcome"),
        }
        match browser_settlement(Some(SignInSettlement::Unconfirmed)) {
            BrowserSettlement::RevokeAndReturn(result) => assert_eq!(json(&result), Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string())),
            _ => panic!("unconfirmed revokes and says so"),
        }
        match browser_settlement(Some(SignInSettlement::Reauthenticated { vault_unlocked: true })) {
            BrowserSettlement::Return(result) => assert_eq!(json(&result), Ok(serde_json::to_value(LoginOutcome::reauthenticated(true)).unwrap())),
            _ => panic!("the same account with its key here returns at once"),
        }
        assert!(matches!(browser_settlement(Some(SignInSettlement::Reauthenticated { vault_unlocked: false })), BrowserSettlement::Install { reauthenticated: true }));
        assert!(matches!(browser_settlement(Some(SignInSettlement::Fresh)), BrowserSettlement::Install { reauthenticated: false }));
        assert!(matches!(browser_settlement(None), BrowserSettlement::Install { reauthenticated: false }));
    }
```

(If step 7 runs, it changes this literal to `pending_changes: Some(3)`; the expected JSON stays the same.)

In `browser_login.rs`'s tests:

```rust
    /// C-S1: no raw code reaches the frontend. The command returns the outcome; neither an error nor an event carries
    /// the mismatch.
    #[test]
    fn the_browser_sign_in_returns_the_shared_outcome_and_no_raw_mismatch() {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/browser_login.rs")).unwrap();
        let production = &source[..source.find("#[cfg(test)]").unwrap_or(source.len())];
        assert!(production.contains("-> Result<crate::LoginOutcome, String>"), "start_browser_login returns LoginOutcome");
        assert!(!production.contains("\"account_mismatch\""), "no raw account_mismatch string, in an error or an event");
        assert!(production.contains("crate::browser_settlement("), "the settlement goes through the shared mapping");
    }
```

Run with filters `a_browser_sign_in` and `the_browser_sign_in_returns_the_shared_outcome`. Expected: compile errors (`browser_settlement` not found), then the source test's assertions.

- [ ] **Step 2: Implement the mapping in `lib.rs`**

After `impl LoginOutcome`'s `account_mismatch`:

```rust
    /// C-S1: a fresh browser sign-in delivers the keys with the session.
    fn browser_signed_in() -> Self {
        Self { vault_unlocked: true, ..Self::signed_in() }
    }
```

Beside `settle_sign_in`:

```rust
/// C-S1: what a browser sign-in does with its settlement. The password path's outcome, for the same settlement, every time.
#[cfg(not(target_os = "windows"))]
enum BrowserSettlement {
    /// Return this; the new session stays (the same account, its key already here).
    Return(Result<LoginOutcome, String>),
    /// Revoke the session just minted for the browser, then return this.
    RevokeAndReturn(Result<LoginOutcome, String>),
    /// Install the handed-over session and key (`apply_session`), then return `browser_installed_outcome`.
    Install { reauthenticated: bool },
}

#[cfg(not(target_os = "windows"))]
fn browser_settlement(settlement: Option<SignInSettlement>) -> BrowserSettlement {
    match settlement {
        Some(SignInSettlement::Reauthenticated { vault_unlocked: true }) => BrowserSettlement::Return(Ok(LoginOutcome::reauthenticated(true))),
        Some(SignInSettlement::AccountMismatch { pending_changes }) => {
            BrowserSettlement::RevokeAndReturn(Ok(LoginOutcome::account_mismatch(pending_changes)))
        }
        Some(SignInSettlement::Unconfirmed) => BrowserSettlement::RevokeAndReturn(Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string())),
        Some(SignInSettlement::Reauthenticated { vault_unlocked: false }) => BrowserSettlement::Install { reauthenticated: true },
        Some(SignInSettlement::Fresh) | None => BrowserSettlement::Install { reauthenticated: false },
    }
}

/// C-S1: the outcome once the browser's session and key are installed: the keys are here either way.
fn browser_installed_outcome(reauthenticated: bool) -> LoginOutcome {
    if reauthenticated { LoginOutcome::reauthenticated(true) } else { LoginOutcome::browser_signed_in() }
}
```

If `SignInSettlement` has other variants at HEAD, the compiler names them; map each to the password path's outcome for the same variant (read `desktop_login`'s match on the settlement).

- [ ] **Step 3: Use it in `browser_login.rs`**

`start_browser_login`'s signature becomes `-> Result<crate::LoginOutcome, String>`, its `Ok(()) => Ok(())` arm `Ok(outcome) => Ok(outcome)`, and `run_handoff` returns `Result<crate::LoginOutcome, String>`. Replace the `let settlement = …; match settlement { … }` block's `match` with:

```rust
        let mut reauthenticated = false;
        match crate::browser_settlement(settlement) {
            crate::BrowserSettlement::Return(result) => {
                emit(app, "done", email.as_ref().map_or_else(|| serde_json::json!({}), |email| serde_json::json!({ "email": email })));
                return result;
            }
            crate::BrowserSettlement::RevokeAndReturn(result) => {
                let _ = crate::revoke_desktop_session(&client, &base_url, &creds.session_token).await;
                return result;
            }
            crate::BrowserSettlement::Install { reauthenticated: same_account } => reauthenticated = same_account,
        }
```

declare `reauthenticated` before the `#[cfg(not(target_os = "windows"))]` block instead (`#[cfg_attr(target_os = "windows", allow(unused_mut))] let mut reauthenticated = false;`) so Windows compiles, and end `run_handoff` with:

```rust
    emit(app, "done", email.map_or_else(|| serde_json::json!({}), |email| serde_json::json!({ "email": email })));
    Ok(crate::browser_installed_outcome(reauthenticated))
```

The old `emit(app, "account_mismatch", …)` line and `return Err("account_mismatch".to_string())` are gone. The `error` event in `start_browser_login`'s `Err` arm stays: it now only ever carries a real error sentence.

- [ ] **Step 4: Run the tests to see them pass**

Run with the filter `-- a_browser_sign_in_reports_the_vault_unlocked a_browser_sign_in_settles_like_the_password_path the_browser_sign_in_returns_the_shared_outcome`, then with filter `login_outcome_json_is_the_frontends_contract` (spec A's pin; its shapes are unchanged).
Expected: `test result: ok. 3 passed; 0 failed`, then `test result: ok. 1 passed; 0 failed`. Paste both lines.

- [ ] **Step 5: Mutation-check**

1. In `browser_settlement`, map `AccountMismatch` to `RevokeAndReturn(Err("account_mismatch".to_string()))`. Expected: `a_browser_sign_in_settles_like_the_password_path`. Revert.
2. Make `browser_signed_in` return `Self::signed_in()`. Expected: `a_browser_sign_in_reports_the_vault_unlocked`. Revert.

- [ ] **Step 6 (only if Task 0 found T11-M5 open): Write the failing count test**

```rust
    /// T11-M5: an unreadable state.db never reads as "0 changes waiting"; the warning then says it could not count.
    #[test]
    fn an_unreadable_state_db_reports_an_unknown_pending_count_never_zero() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(state_paths::STATE_DB_FILENAME), b"this is not a database").unwrap();
        assert_eq!(pending_changes_count(&LocalSources::for_test(dir.path())), None);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(pending_changes_count(&LocalSources::for_test(empty.path())), Some(0), "no database yet: nothing waits");
        assert_eq!(
            serde_json::to_value(LoginOutcome::account_mismatch(None)).unwrap()["account_mismatch"],
            serde_json::json!({ "pending_changes": null })
        );
    }
```

Run it. Expected: compile error (`pending_changes_count` returns `u64`).

- [ ] **Step 7 (only if M5 open): Implement**

```rust
/// Changes waiting to upload, for the switch warning. `None` when they cannot be counted (an unreadable `state.db`):
/// the warning then says it could not count them, never "0".
#[cfg(not(target_os = "windows"))]
fn pending_changes_count(sources: &LocalSources) -> Option<u64> {
    match sources.state_db() {
        Ok(Some(db)) => db.queue_diagnostics(now_unix_seconds()).ok().map(|queue| u64::try_from(queue.queued).unwrap_or(u64::MAX)),
        Ok(None) => Some(0),
        Err(_) => None,
    }
}
```

`AccountMismatchDto { pending_changes: Option<u64> }`, `LoginOutcome::account_mismatch(pending_changes: Option<u64>)`, `SignInSettlement::AccountMismatch { pending_changes: Option<u64> }`, and `LocalFacts.pending_changes: Option<u64>`. In `gather_local_facts`, the trace becomes `queued_or_staged: pending_changes.is_none_or(|n| n > 0) || queued_or_staged_present(sources)` (an uncountable queue is a trace). Fix every compile error the type change raises (each is a `u64` that is now `Option<u64>`; never `unwrap_or(0)` it), and change `a_browser_sign_in_settles_like_the_password_path`'s literal to `pending_changes: Some(3)`.

- [ ] **Step 8 (only if M5 open): Run, mutation-check, and confirm the frontend contract**

Run the M5 test, `login_outcome_json_is_the_frontends_contract` and the whole R8 module. Mutation: make the `Err(_)` arm return `Some(0)`; expect `an_unreadable_state_db_reports_an_unknown_pending_count_never_zero`; revert. Write in the Notes: "M5 done in Rust; Task 16 renders `pending_changes: null`."

- [ ] **Step 9: Commit**

```bash
cd $WT/src-tauri && SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/t12-all.log 2>&1; echo "rc=$?"
rm -rf "$SCRATCH"; /usr/bin/grep "test result:" $EVID/t12-all.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t12-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/browser_login.rs src-tauri/src/lib.rs
git commit -m "desktop: the browser sign-in returns the same outcome as the password sign-in" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/browser_login.rs src-tauri/src/lib.rs
git show --stat HEAD
```

Expected: every binary `0 failed`; on macOS the lib is `B_lib` + 145 (+ 1 if steps 6–8 ran), the running total in Global Constraints.

---

## Task 13: The macOS launch — login or manual, the settle, the one window rule (spec §6.1, §6.2)

**Lane R.** macOS only in behaviour; Windows and Linux keep today's startup, and their pin tests keep their assertions.

**Files:**
- Create: `src-tauri/src/launch_kind.rs`, `src-tauri/macos/LaunchEvent.m`
- Modify: `src-tauri/build.rs` (compile `LaunchEvent.m` into the same static library)
- Modify: `src-tauri/src/surfaces/policy.rs` (`StartupSurface`, `startup_surface`, tests)
- Modify: `src-tauri/src/lib.rs` (`mod launch_kind;`; `ONBOARDING_TITLE`; `open_reauth_window_impl`; `show_macos_settings_window_on_account`; `open_surface`; `open_startup_surface_after_settle`; `show_settings_on_account` command; `setup()`; `.build(…).run(…)` with `RunEvent::Reopen`; tests)

**Interfaces:**
- Consumes: `derive::{AccountView, Screen}` (Task 5), `policy::settle_deadline` (Task 6), `AccountViewHandle::{settled, state}` (Task 10), `AppState.account_view` (Task 11); spec A's `open_onboarding_window_impl`, `open_reauth_window`, `show_macos_settings_window`, `show_compact_app_window_with_nav`, `DesktopConfig`.
- Produces:
  - `crate::launch_kind::{LaunchKind { Login, Manual }, launch_kind(Option<bool>) -> LaunchKind, from_bridge(i32) -> Option<bool>, current() -> LaunchKind}`
  - ObjC `int beebeeb_launched_as_login_item(void)` (1 the `keyAELaunchedAsLogInItem` parameter is present, 2 `keyAEPropData` carries the enum `'lgit'`, 0 neither, -1 unreadable)
  - `surfaces::policy::StartupSurface::{Onboarding, MainWindow, AccountWindow, SettingsAccount, Nothing}`; `startup_surface(platform: Platform, no_sync_root: bool, launch: LaunchKind, account: Option<&AccountView>) -> StartupSurface`
  - `lib.rs` (macOS): `fn open_surface(app: &tauri::AppHandle, surface: StartupSurface, view: &AccountView)`, `fn show_macos_settings_window_on_account(app: &tauri::AppHandle)`, `fn open_reauth_window_impl(app: &tauri::AppHandle) -> Result<(), String>`; command `show_settings_on_account() -> Result<(), String>` (Task 17's hand-over)
  - the macOS account window's URL carries `platform=macos`: `index.html?window=onboarding&platform=macos`, and in reauth mode `index.html?window=onboarding&mode=reauth&platform=macos` (Task 17's `main.tsx` routes on it). Linux keeps `index.html?window=onboarding` and `…&mode=reauth`.

- [ ] **Step 1: `launch_kind`, test first**

`src-tauri/src/launch_kind.rs`:

```rust
//! Spec 2026-10-07 C-W1: was this launch started by the login items, or by a person? Read once, at launch, from the
//! open-application Apple event: its `keyAELaunchedAsLogInItem` (`'lgit'`) parameter (macOS SDK 27.0
//! `AERegistry.h:1052`: "If present in a kAEOpenApplication event, application was launched as a login item"), or its
//! `keyAEPropData` (`'prdt'`, `AERegistry.h:364`) parameter carrying the enum `'lgit'`, the form some login-item
//! detectors read. Either one: a login launch. Neither, unreadable, or a launch by any other mechanism (Finder, the
//! Dock, Spotlight, the updater's relaunch, which spawns the binary directly): manual, so the window shows. Failing
//! safe costs one window at login.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKind {
    Login,
    Manual,
}

/// `Some(true)`: the parameter is present. `Some(false)`: an open-application event without it. `None`: unreadable.
pub fn launch_kind(lgit_present: Option<bool>) -> LaunchKind {
    todo!("Task 13 step 3")
}

/// The bridge's answer (`beebeeb_launched_as_login_item`): 1 or 2 present (the keyword, or `keyAEPropData`), 0 absent,
/// anything else unreadable.
pub fn from_bridge(answer: i32) -> Option<bool> {
    todo!("Task 13 step 3")
}

/// Read the launch now. Call it first thing in `setup()`, while the open-application event is current.
pub fn current() -> LaunchKind {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn beebeeb_launched_as_login_item() -> i32;
        }
        // SAFETY: a plain C function with no arguments; it reads the current Apple event inside an autorelease pool.
        let answer = unsafe { beebeeb_launched_as_login_item() };
        // Which field the OS set is recorded on the Mac (Task 23's login rung).
        tracing::info!(bridge = answer, "launch kind read from the open-application event");
        launch_kind(from_bridge(answer))
    }
    #[cfg(not(target_os = "macos"))]
    {
        LaunchKind::Manual
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_parameter_present_is_a_login_launch() {
        assert_eq!(launch_kind(Some(true)), LaunchKind::Login);
        assert_eq!(launch_kind(Some(false)), LaunchKind::Manual, "absent");
        assert_eq!(launch_kind(None), LaunchKind::Manual, "unreadable or another mechanism");
    }

    #[test]
    fn the_bridge_answer_maps_to_present_absent_or_unreadable() {
        assert_eq!(from_bridge(1), Some(true));
        assert_eq!(from_bridge(2), Some(true), "keyAEPropData carrying 'lgit'");
        assert_eq!(from_bridge(0), Some(false));
        assert_eq!(from_bridge(-1), None);
        assert_eq!(from_bridge(7), None);
    }

    /// The bridge links and answers in a process that no login item started (a test binary has no Apple event).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_test_process_is_a_manual_launch() {
        assert_eq!(current(), LaunchKind::Manual);
    }
}
```

Add `mod launch_kind;` to `lib.rs`'s module list. Run with filter `launch_kind`. Expected: on macOS a link error (`beebeeb_launched_as_login_item` undefined) until step 2; paste it.

- [ ] **Step 2: The Objective-C reader and its build**

`src-tauri/macos/LaunchEvent.m`:

```objc
// Spec 2026-10-07 C-W1: was this launch started by the login items? Read while the app finishes launching (the Rust
// side calls this first thing in setup()). Foundation only: the event codes are written as four-character constants,
// with the macOS SDK 27.0 header lines they come from.
#import <Foundation/Foundation.h>

static const FourCharCode kBeebeebCoreEventClass = 'aevt';       // kCoreEventClass, AppleEvents.h:58
static const FourCharCode kBeebeebOpenApplication = 'oapp';      // kAEOpenApplication, AppleEvents.h:63
static const FourCharCode kBeebeebLaunchedAsLogInItem = 'lgit';  // keyAELaunchedAsLogInItem, AERegistry.h:1052
static const FourCharCode kBeebeebPropData = 'prdt';             // keyAEPropData, AERegistry.h:364

/// 1: the keyAELaunchedAsLogInItem parameter is present, as the SDK documents it. 2: keyAEPropData carries the enum
/// 'lgit', the form some login-item detectors read. Either is a login launch. 0: an open-application event with
/// neither. -1: no current event, or another event, so nothing to read (the caller counts that as a manual launch).
int beebeeb_launched_as_login_item(void) {
    @autoreleasepool {
        NSAppleEventDescriptor *event = [[NSAppleEventManager sharedAppleEventManager] currentAppleEvent];
        if (event == nil || event.eventClass != kBeebeebCoreEventClass || event.eventID != kBeebeebOpenApplication) {
            return -1;
        }
        if ([event paramDescriptorForKeyword:kBeebeebLaunchedAsLogInItem] != nil) {
            return 1;
        }
        NSAppleEventDescriptor *property = [event paramDescriptorForKeyword:kBeebeebPropData];
        return (property != nil && property.enumCodeValue == kBeebeebLaunchedAsLogInItem) ? 2 : 0;
    }
}
```

Before you write it, check the four constants against the SDK on this Mac and paste the lines into the Notes (they were checked on 2026-10-07 against SDK 27.0):

```bash
H=$(xcrun --show-sdk-path)/System/Library/Frameworks/CoreServices.framework/Frameworks/AE.framework/Headers
/usr/bin/grep -n "keyAELaunchedAsLogInItem *=\|keyAEPropData *=" $H/AERegistry.h > $EVID/t13-sdk-keywords.txt
/usr/bin/grep -n "kCoreEventClass *=\|kAEOpenApplication *=" $H/AppleEvents.h >> $EVID/t13-sdk-keywords.txt; cat $EVID/t13-sdk-keywords.txt
```

Expected: `'lgit'`, `'prdt'`, `'aevt'`, `'oapp'`. A different value stops the task.

`src-tauri/build.rs`: compile it beside the bridge and archive both objects. Replace `build_macos_file_provider_bridge` with:

```rust
fn build_macos_file_provider_bridge(target: &str) {
    use std::path::PathBuf;
    use std::process::Command;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let archive = out_dir.join("libbeebeeb_file_provider_bridge.a");
    let clang_target = match target {
        "aarch64-apple-darwin" => "arm64-apple-macos14.0",
        "x86_64-apple-darwin" | "x86_64h-apple-darwin" => "x86_64-apple-macos14.0",
        _ => "arm64-apple-macos14.0",
    };

    // FileProviderBridge.m (spec A) and LaunchEvent.m (spec 2026-10-07 C-W1), in one static library.
    let mut objects = Vec::new();
    for source in ["macos/FileProviderBridge.m", "macos/LaunchEvent.m"] {
        let stem = std::path::Path::new(source).file_stem().and_then(|s| s.to_str()).expect("a file stem");
        let object = out_dir.join(format!("{stem}.o"));
        let status = Command::new("xcrun")
            .args(["clang", "-target", clang_target, "-fobjc-arc", "-fmodules", "-mmacosx-version-min=14.0", "-c", source, "-o"])
            .arg(&object)
            .status()
            .unwrap_or_else(|e| panic!("run clang for {source}: {e}"));
        assert!(status.success(), "clang failed for {source}");
        println!("cargo:rerun-if-changed={source}");
        objects.push(object);
    }

    let _ = std::fs::remove_file(&archive);
    let status = Command::new("ar").arg("crs").arg(&archive).args(&objects).status().expect("archive the macOS objects");
    assert!(status.success(), "ar failed for the macOS objects");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=beebeeb_file_provider_bridge");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=FileProvider");
}
```

Run with filter `launch_kind`. Expected: the build succeeds; the tests fail only on `todo!()`.

- [ ] **Step 3: Implement the two pure functions**

```rust
pub fn launch_kind(lgit_present: Option<bool>) -> LaunchKind {
    if lgit_present == Some(true) { LaunchKind::Login } else { LaunchKind::Manual }
}

pub fn from_bridge(answer: i32) -> Option<bool> {
    match answer {
        1 | 2 => Some(true),
        0 => Some(false),
        _ => None,
    }
}
```

Run with filter `launch_kind`. Expected: `test result: ok. 3 passed; 0 failed` on macOS (2 elsewhere).

Mutation-check (paste each failure, revert, paste the green):
1. In `launch_kind`, write `if lgit_present != Some(false)`. Expected: `only_the_parameter_present_is_a_login_launch` ("unreadable or another mechanism").
2. In `from_bridge`, write `1 => Some(true),` (drop `| 2`). Expected: `the_bridge_answer_maps_to_present_absent_or_unreadable` ("keyAEPropData carrying 'lgit'").
3. In `LaunchEvent.m`, make the function `return 1;` first. Expected: `a_test_process_is_a_manual_launch` (left `Login`).

- [ ] **Step 4: Write the `startup_surface` tests**

In `surfaces/policy.rs`'s `mod tests`, add (and give every existing `startup_surface(platform, no_sync_root)` call the two new arguments `LaunchKind::Manual, None`, keeping each assertion as it is):

```rust
    // ── Spec 2026-10-07 C-W4, C-W5, C-W9: the macOS row ────────────────────────────────────────────────

    use crate::account_view::derive::{AccountState, AccountView, Screen};
    use crate::launch_kind::LaunchKind;

    fn view(state: AccountState, screen: Screen) -> AccountView {
        AccountView { state, screen, busy: state == AccountState::Checking, ..AccountView::signed_out(false) }
    }

    fn manual(view: &AccountView, no_sync_root: bool) -> StartupSurface {
        startup_surface(Platform::Macos, no_sync_root, LaunchKind::Manual, Some(view))
    }

    #[test]
    fn manual_launch_signed_out_opens_sign_in() {
        let v = view(AccountState::SignedOut, Screen::SignIn);
        assert_eq!((manual(&v, false), v.screen), (StartupSurface::AccountWindow, Screen::SignIn));
    }

    #[test]
    fn manual_launch_session_ended_opens_sign_in_again() {
        let v = view(AccountState::SessionEnded, Screen::SignInAgain);
        assert_eq!((manual(&v, false), v.screen), (StartupSurface::AccountWindow, Screen::SignInAgain));
    }

    #[test]
    fn manual_launch_update_required_opens_update() {
        let v = view(AccountState::UpdateRequired, Screen::Update);
        assert_eq!((manual(&v, false), v.screen), (StartupSurface::AccountWindow, Screen::Update));
    }

    #[test]
    fn manual_launch_locked_with_keychain_key_opens_settings() {
        assert_eq!(manual(&view(AccountState::Locked, Screen::KeychainUnlock), false), StartupSurface::SettingsAccount);
    }

    #[test]
    fn manual_launch_locked_without_key_opens_recovery_phrase() {
        let v = view(AccountState::Locked, Screen::RecoveryPhrase);
        assert_eq!((manual(&v, false), v.screen), (StartupSurface::AccountWindow, Screen::RecoveryPhrase));
    }

    #[test]
    fn manual_launch_no_plan_opens_choose_a_plan() {
        let v = view(AccountState::NoPlan, Screen::ChoosePlan);
        assert_eq!((manual(&v, false), v.screen), (StartupSurface::AccountWindow, Screen::ChoosePlan));
    }

    #[test]
    fn manual_launch_still_checking_after_settle_opens_busy() {
        let v = view(AccountState::Checking, Screen::Busy);
        assert_eq!((manual(&v, false), v.screen, v.busy), (StartupSurface::AccountWindow, Screen::Busy, true));
    }

    #[test]
    fn manual_launch_ready_without_sync_root_opens_onboarding() {
        assert_eq!(manual(&view(AccountState::Ready, Screen::Nothing), true), StartupSurface::Onboarding);
    }

    #[test]
    fn manual_launch_ready_with_sync_root_opens_compact_window() {
        assert_eq!(manual(&view(AccountState::Ready, Screen::Nothing), false), StartupSurface::MainWindow);
    }

    #[test]
    fn login_launch_never_opens_a_window() {
        for (state, screen) in [
            (AccountState::Checking, Screen::Busy),
            (AccountState::SignedOut, Screen::SignIn),
            (AccountState::SessionEnded, Screen::SignInAgain),
            (AccountState::UpdateRequired, Screen::Update),
            (AccountState::Locked, Screen::KeychainUnlock),
            (AccountState::Locked, Screen::RecoveryPhrase),
            (AccountState::NoPlan, Screen::ChoosePlan),
            (AccountState::Ready, Screen::Nothing),
        ] {
            for no_sync_root in [true, false] {
                let v = view(state, screen);
                assert_eq!(
                    startup_surface(Platform::Macos, no_sync_root, LaunchKind::Login, Some(&v)),
                    StartupSurface::Nothing,
                    "{state:?} / {screen:?} / no_sync_root {no_sync_root}"
                );
            }
        }
    }

    #[test]
    fn windows_and_linux_ignore_the_launch_kind_and_the_account_view() {
        let signed_out = view(AccountState::SignedOut, Screen::SignIn);
        for platform in [Platform::Windows, Platform::Linux] {
            for launch in [LaunchKind::Login, LaunchKind::Manual] {
                assert_eq!(startup_surface(platform, true, launch, Some(&signed_out)), StartupSurface::Onboarding, "{platform:?}");
                assert_eq!(startup_surface(platform, false, launch, Some(&signed_out)), StartupSurface::MainWindow, "{platform:?}");
            }
        }
    }
```

Run with filter `surfaces::policy`. Expected: compile errors (the new variants and parameters); paste them.

- [ ] **Step 5: Implement `startup_surface`**

```rust
use crate::account_view::derive::{AccountView, Screen};
use crate::launch_kind::LaunchKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupSurface {
    /// First launch (no sync root configured yet): the onboarding window.
    Onboarding,
    /// Already configured: the platform's main window, shown after startup. On macOS, the compact window on its
    /// Finder location view (spec 2026-10-07 §11).
    MainWindow,
    /// macOS (spec 2026-10-07 C-W4): the account window, on the account view's screen.
    AccountWindow,
    /// macOS (C-W7): the Settings window on its Account tab, where the Keychain unlock is.
    SettingsAccount,
    /// macOS (C-W5): a login launch opens nothing.
    Nothing,
}

/// What the app opens after launch. Windows and Linux: today's rule, onboarding on first run, else the main window
/// (their pin tests below are unchanged). macOS (spec 2026-10-07 §6.2): a login launch opens nothing; a manual one opens
/// the account window in any state but Ready, Settings for the Keychain unlock, and today's rule in Ready.
pub fn startup_surface(platform: Platform, no_sync_root: bool, launch: LaunchKind, account: Option<&AccountView>) -> StartupSurface {
    let today = if no_sync_root { StartupSurface::Onboarding } else { StartupSurface::MainWindow };
    match platform {
        Platform::Windows | Platform::Linux => today,
        Platform::Macos => match (launch, account) {
            (LaunchKind::Login, _) => StartupSurface::Nothing,
            (LaunchKind::Manual, None) => today,
            (LaunchKind::Manual, Some(view)) => match view.screen {
                Screen::Nothing => today,
                Screen::KeychainUnlock => StartupSurface::SettingsAccount,
                _ => StartupSurface::AccountWindow,
            },
        },
    }
}
```

Update the module comment's "Today:" sentence for `startup_surface` to say the macOS row changed in spec 2026-10-07. Run with filter `surfaces::policy`. Expected: `test result: ok. N passed; 0 failed`, N = the filter's count before this task + 11.

Mutation-check (paste each failure, revert, paste the green):
1. `(LaunchKind::Login, _) => today`. Expected: `login_launch_never_opens_a_window`.
2. `Screen::KeychainUnlock => StartupSurface::AccountWindow`. Expected: `manual_launch_locked_with_keychain_key_opens_settings`.
3. `Screen::Nothing => StartupSurface::AccountWindow`. Expected: `manual_launch_ready_without_sync_root_opens_onboarding` and `manual_launch_ready_with_sync_root_opens_compact_window`.
4. `_ => today` in the last arm. Expected: the six `manual_launch_…` tests that expect `AccountWindow`.
5. Route `Platform::Windows | Platform::Linux` into the macOS `match`. Expected: `windows_and_linux_ignore_the_launch_kind_and_the_account_view`.

- [ ] **Step 6: Wire the launch in `lib.rs`**

Window title (C-W8), beside `open_onboarding_window_impl`:

```rust
/// Spec 2026-10-07 C-W8: on macOS the account window is "Sign in to Beebeeb"; Linux keeps "Welcome to Beebeeb".
#[cfg(target_os = "macos")]
const ONBOARDING_TITLE: &str = "Sign in to Beebeeb";
#[cfg(not(target_os = "macos"))]
const ONBOARDING_TITLE: &str = "Welcome to Beebeeb";
```

and replace `.title("Welcome to Beebeeb")` with `.title(ONBOARDING_TITLE)` in `open_onboarding_window_impl` and in `open_reauth_window`.

The macOS account window is tagged the way the compact window already is (`?platform=macos`), so the frontend picks it before any snapshot has loaded. In `open_onboarding_window_impl`, the non-Windows tuple becomes:

```rust
    #[cfg(not(target_os = "windows"))]
    let (label, url, width, height) = ("onboarding", ONBOARDING_URL, 860.0_f64, 640.0_f64);
```

with

```rust
/// Spec 2026-10-07 C-W8: the account window (macOS) and the onboarding window (Linux) share the label `onboarding`; the
/// macOS URL carries `platform=macos` so `main.tsx` mounts the account window.
#[cfg(target_os = "macos")]
const ONBOARDING_URL: &str = "index.html?window=onboarding&platform=macos";
#[cfg(target_os = "macos")]
const REAUTH_SEARCH: &str = "?window=onboarding&mode=reauth&platform=macos";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const ONBOARDING_URL: &str = "index.html?window=onboarding";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const REAUTH_SEARCH: &str = "?window=onboarding&mode=reauth";
```

and in `open_reauth_window_impl`, `const URL` becomes `let url = format!("index.html{REAUTH_SEARCH}");` and the `eval` string `format!("window.location.search = '{REAUTH_SEARCH}'")`.

Split `open_reauth_window` so Rust can call it: move its body into `pub(crate) fn open_reauth_window_impl(app: &tauri::AppHandle) -> Result<(), String>` (same code, `app` borrowed), and make the command `open_reauth_window_impl(&app)`.

That breaks a spec A pin, which this task updates (plan review M9): `the_commands_and_the_window_the_frontend_calls_exist` reads `body_between(&production, "async fn open_reauth_window(", "\n}\n")` and asserts `const URL: &str = "index.html?window=onboarding&mode=reauth";` and the `eval` string in it. Change that test so that:
- `window` is read from `"fn open_reauth_window_impl("`, and it asserts `window.contains("let url = format!(\"index.html{REAUTH_SEARCH}\");")` and `window.contains("format!(\"window.location.search = '{REAUTH_SEARCH}'\")")` in place of the two literal strings (the values are pinned by `the_macos_account_window_is_tagged_for_the_frontend` below);
- the command's own body (`body_between(&production, "async fn open_reauth_window(", "\n}\n")`) contains `open_reauth_window_impl(&app)`;
- its Windows-arm assertion reads the impl's body, unchanged otherwise.
Run it (filter `the_commands_and_the_window_the_frontend_calls_exist`) before and after the edit and paste both lines: red after the split, green after the test change.

The Settings hand-over (C-W7, Review Focus 5):

```rust
/// Spec 2026-10-07 C-W7: the Keychain unlock is the Settings window's Account tab, so that is the tab it opens on, not the
/// one it last showed. The query is the window's own (`tauri.conf.json`, label `macos-settings`) plus `tab=account`,
/// which `settingsTabFromSearch` reads.
#[cfg(target_os = "macos")]
fn show_macos_settings_window_on_account(app: &tauri::AppHandle) {
    show_macos_settings_window(app);
    if let Some(win) = app.get_webview_window("macos-settings") {
        let _ = win.eval(
            "if (new URLSearchParams(window.location.search).get('tab') !== 'account') { window.location.search = '?window=settings-v2&platform=macos&tab=account' }",
        );
    }
}

/// Task 17's hand-over: the account window asks for Settings on its Account tab, then hides itself.
#[tauri::command]
fn show_settings_on_account(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        show_macos_settings_window_on_account(&app);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Only available on macOS.".to_string())
    }
}
```

(add `show_settings_on_account` to `generate_handler![…]`).

The one window rule (C-W3/C-W4, used by the launch, by Reopen, and by Task 14's click):

```rust
/// Spec 2026-10-07 §6.2: open what a launch or a click asks for, on the account view's screen.
#[cfg(target_os = "macos")]
fn open_surface(app: &tauri::AppHandle, surface: surfaces::policy::StartupSurface, view: &account_view::derive::AccountView) {
    use surfaces::policy::StartupSurface;
    let result = match surface {
        StartupSurface::Nothing => Ok(()),
        StartupSurface::AccountWindow if view.screen == account_view::derive::Screen::SignInAgain => open_reauth_window_impl(app),
        StartupSurface::Onboarding | StartupSurface::AccountWindow => open_onboarding_window_impl(app),
        StartupSurface::MainWindow => {
            show_compact_app_window_with_nav(app, Some("finder"));
            Ok(())
        }
        StartupSurface::SettingsAccount => {
            show_macos_settings_window_on_account(app);
            Ok(())
        }
    };
    if let Err(error) = result {
        tracing::warn!(%error, ?surface, "could not open the window the account state asks for");
    }
}

/// Spec 2026-10-07 C-W4/C-W5 (macOS): once the restore probe has finished, wait for the account state to leave
/// Checking (at most 5 s), then open what `startup_surface` says. A login launch opens nothing.
#[cfg(target_os = "macos")]
async fn open_startup_surface_after_settle(app: &tauri::AppHandle, launch: launch_kind::LaunchKind) {
    let deadline = tokio::time::Instant::from_std(account_view::policy::settle_deadline(std::time::Instant::now()));
    let Some(handle) = app.state::<AppState>().account_view.get().cloned() else {
        return;
    };
    let view = handle.settled(deadline).await;
    let no_sync_root = DesktopConfig::load().map(|c| c.sync_root.is_none()).unwrap_or(true);
    let surface = surfaces::policy::startup_surface(surfaces::policy::Platform::Macos, no_sync_root, launch, Some(&view));
    open_surface(app, surface, &view);
}

/// C-W2: opening the app while it runs (Finder, the Dock, Spotlight) is a manual launch, with no settle.
#[cfg(target_os = "macos")]
fn open_surface_on_reopen(app: &tauri::AppHandle) {
    let Some(view) = app.state::<AppState>().account_view.get().map(|handle| handle.state()) else {
        return;
    };
    let no_sync_root = DesktopConfig::load().map(|c| c.sync_root.is_none()).unwrap_or(true);
    let surface = surfaces::policy::startup_surface(surfaces::policy::Platform::Macos, no_sync_root, launch_kind::LaunchKind::Manual, Some(&view));
    open_surface(app, surface, &view);
}
```

In `setup()`: first statement of the closure, `let launch = launch_kind::current();`. In the restore task, after `drop(restore_finished);` (Task 11) and the Finder `Launch` trigger, so spec A's `the_launch_trigger_follows_the_startup_restore` (at most 400 characters from the restore call to that trigger) is unaffected:

```rust
                    #[cfg(target_os = "macos")]
                    open_startup_surface_after_settle(&h, launch).await;
```

(`launch` is `Copy`; move it into the task.) The existing ~300 ms `startup_surface` block becomes non-macOS only: wrap it in `#[cfg(not(target_os = "macos"))]` and change its call to `surfaces::policy::startup_surface(surfaces::policy::Platform::current(), no_sync_root, launch_kind::LaunchKind::Manual, None)`. On non-macOS add `let _ = launch;` where needed.

`RunEvent::Reopen`: replace `.run(tauri::generate_context!()).expect("error while running Beebeeb desktop");` with

```rust
        .build(tauri::generate_context!())
        .expect("error while building Beebeeb desktop")
        .run(|app, event| {
            // Spec 2026-10-07 C-W2: reopening the running app is a manual launch.
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen { .. } = event {
                    open_surface_on_reopen(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
```

- [ ] **Step 7: Source tests for the wiring**

In `finder_setup_command_tests`:

```rust
    /// C-W4: the settle starts after the restore probe has finished (the restore returns after it), and the macOS
    /// launch goes through `startup_surface` with the launch kind read at the start of setup().
    #[test]
    fn settle_starts_after_the_restore_probe() {
        let production = production_source();
        let restore = production.find("restore_session_on_startup(&h).await").expect("the restore");
        let settle = production.find("open_startup_surface_after_settle(&h, launch).await").expect("the settled launch");
        assert!(restore < settle);
        let body = body_between(&production, "async fn open_startup_surface_after_settle(", "\n}\n");
        assert!(body.contains("settle_deadline(std::time::Instant::now())") && body.contains(".settled(deadline)"));
        assert!(production.contains("let launch = launch_kind::current();"));
    }

    /// Review Focus 5, plan "Spec issues" 12: the Keychain unlock opens Settings on its Account tab.
    #[test]
    fn the_keychain_unlock_opens_settings_on_the_account_tab() {
        let production = production_source();
        let body = body_between(&production, "fn show_macos_settings_window_on_account(", "\n}\n");
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let url = config["app"]["windows"].as_array().unwrap().iter().find(|w| w["label"] == "macos-settings").unwrap()["url"].as_str().unwrap().to_string();
        let query = url.split_once('?').map(|(_, q)| q).expect("the window's query");
        assert!(body.contains(&format!("?{query}&tab=account")), "{body}");
        let open = body_between(&production, "fn open_surface(", "\n}\n");
        assert!(open.contains("StartupSurface::SettingsAccount => {") && open.contains("show_macos_settings_window_on_account(app)"));
    }

    #[test]
    fn the_account_window_is_titled_sign_in_on_macos_only() {
        let production = production_source();
        assert!(production.contains("const ONBOARDING_TITLE: &str = \"Sign in to Beebeeb\";"));
        assert!(production.contains("const ONBOARDING_TITLE: &str = \"Welcome to Beebeeb\";"));
        for function in ["fn open_onboarding_window_impl(", "fn open_reauth_window_impl("] {
            let body = body_between(&production, function, "\n}\n");
            assert!(body.contains(".title(ONBOARDING_TITLE)") && !body.contains(".title(\"Welcome"), "{function}");
        }
    }

    #[test]
    fn the_macos_account_window_is_tagged_for_the_frontend() {
        let production = production_source();
        assert!(production.contains("const ONBOARDING_URL: &str = \"index.html?window=onboarding&platform=macos\";"));
        assert!(production.contains("const REAUTH_SEARCH: &str = \"?window=onboarding&mode=reauth&platform=macos\";"));
        assert!(production.contains("const REAUTH_SEARCH: &str = \"?window=onboarding&mode=reauth\";"), "Linux keeps spec A's search");
    }

    #[test]
    fn reopening_the_running_app_is_a_manual_launch() {
        let production = production_source();
        assert!(production.contains("if let tauri::RunEvent::Reopen { .. } = event {"));
        let body = body_between(&production, "fn open_surface_on_reopen(", "\n}\n");
        assert!(body.contains("LaunchKind::Manual"));
    }
```

(The path in `include_str!` is relative to `src/lib.rs`; adjust it if the test module lives elsewhere.) Run with the filter `-- settle_starts_after_the_restore_probe the_keychain_unlock_opens_settings_on_the_account_tab the_account_window_is_titled_sign_in_on_macos_only the_macos_account_window_is_tagged_for_the_frontend reopening_the_running_app_is_a_manual_launch`. Expected: `test result: ok. 5 passed; 0 failed`.

Mutation-check (paste each failure, revert, paste the green):
1. `tab=account` → `tab=general` in `show_macos_settings_window_on_account`. Expected: `the_keychain_unlock_opens_settings_on_the_account_tab`.
2. Move `open_startup_surface_after_settle(&h, launch).await` above `restore_session_on_startup(&h).await`. Expected: `settle_starts_after_the_restore_probe`.
3. Make the macOS `ONBOARDING_TITLE` `"Welcome to Beebeeb"`. Expected: `the_account_window_is_titled_sign_in_on_macos_only`.
4. Drop `&platform=macos` from the macOS `ONBOARDING_URL`. Expected: `the_macos_account_window_is_tagged_for_the_frontend`.
5. Pass `LaunchKind::Login` in `open_surface_on_reopen`. Expected: `reopening_the_running_app_is_a_manual_launch`.

- [ ] **Step 8: Run everything and commit**

```bash
cd $WT/src-tauri && SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/t13-all.log 2>&1; echo "rc=$?"
rm -rf "$SCRATCH"; /usr/bin/grep "test result:" $EVID/t13-all.log
$LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t13-check.log 2>&1; echo "rc=$?"
cd $WT && git add src-tauri/src/launch_kind.rs src-tauri/macos/LaunchEvent.m src-tauri/build.rs src-tauri/src/surfaces/policy.rs src-tauri/src/lib.rs
git commit -m "desktop: on macOS, open the window the account state asks for, and nothing at login" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/launch_kind.rs src-tauri/macos/LaunchEvent.m src-tauri/build.rs src-tauri/src/surfaces/policy.rs src-tauri/src/lib.rs
git show --stat HEAD
```

Expected: every binary `0 failed`; on macOS the lib is `B_lib` + 164 (+ 1 if Task 12's M5 steps ran); `the_commands_and_the_window_the_frontend_calls_exist` and `the_launch_trigger_follows_the_startup_restore` (spec A) pass.

---

## Task 14: The menu-bar icon — the crossed b, its tooltip, the click and the menu (spec §8, C-W3, C-W10, C-W11)

**Lane R.** One pure function decides the presentation; the runtime applies it only when it changes.

**Files:**
- Create: `src-tauri/src/tray_presentation.rs`
- Modify: `src-tauri/Cargo.toml` (tauri `image-ico`)
- Modify: `src-tauri/src/lib.rs` (`mod tray_presentation;`; the embedded icons; `TRAY_STATE`; `apply_tray_presentation`; `attach_tray_status_listener`; `setup_tray`'s click handler; tests)
- Modify: `src-tauri/src/account_view/app_ports.rs` (`AppPublisher::publish` posts the presentation to the main thread)
- Modify: `src-tauri/src/account_view/driver.rs` (tests: the crossed b on the two published sign-out views)

**Interfaces:**
- Consumes: `derive::{AccountView, Screen, SessionKind}` (Task 5), `open_surface` and `show_macos_settings_window_on_account` (Task 13), `surfaces::policy::StartupSurface`, the icon files from Task 1; spec A's `setup_tray`, `build_tray_menu`, `attach_tray_status_listener`, `engine_status::tray_tooltip`.
- Produces (`crate::tray_presentation`):
  - `SIGNED_OUT_TOOLTIP = "Beebeeb: Signed out"`, `SIGN_IN_AGAIN_TOOLTIP = "Beebeeb: Sign in again"`, `OFFLINE_TOOLTIP = "Beebeeb: Offline"`
  - `enum TrayIcon { Normal, Crossed }`, `enum ClickAction { Unchanged, OpenAccountWindow, OpenSettingsAccount, ToggleCompactWindow }`, `enum Button { Left, Right }`
  - `struct TrayPresentation { icon, tooltip: Option<&'static str>, click, menu_attached: bool, both_buttons: bool }`
  - `fn present(view: &AccountView, platform: Platform) -> TrayPresentation`, `fn click_action(&TrayPresentation, Button) -> Option<ClickAction>`, `fn tooltip_text(&TrayPresentation, engine_tooltip: Option<&str>) -> String`, `fn engine_may_set_tooltip(applied: Option<&TrayPresentation>) -> bool`
  - `lib.rs`: `pub(crate) fn apply_tray_presentation(app: &tauri::AppHandle, view: &AccountView)`

- [ ] **Step 1: Record the lockfile before the feature change**

```bash
cd $WT/src-tauri && /usr/bin/grep -c '^\[\[package\]\]' Cargo.lock > $EVID/t14-lock-before.txt; shasum -a 256 Cargo.lock >> $EVID/t14-lock-before.txt; cat $EVID/t14-lock-before.txt
```

- [ ] **Step 2: `tray_presentation.rs` with its tests and `todo!()` bodies**

```rust
//! Spec 2026-10-07 §8: the menu-bar icon, its tooltip, what a click does and whether the menu is attached, from the
//! account view alone. The crossed b follows the session and connectivity (C-I1), never the window's state: an update
//! that is required while signed out still shows "Signed out".

use crate::account_view::derive::{AccountView, Screen, SessionKind};
use crate::surfaces::policy::Platform;

pub const SIGNED_OUT_TOOLTIP: &str = "Beebeeb: Signed out";
pub const SIGN_IN_AGAIN_TOOLTIP: &str = "Beebeeb: Sign in again";
pub const OFFLINE_TOOLTIP: &str = "Beebeeb: Offline";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIcon {
    Normal,
    Crossed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    /// Windows and Linux: today's clicks (C-I6).
    Unchanged,
    /// macOS: the account window on the view's screen (C-W3).
    OpenAccountWindow,
    /// macOS C-W7: Settings on its Account tab.
    OpenSettingsAccount,
    /// macOS Ready: today's toggle of the compact window, which lands on the Finder location view.
    ToggleCompactWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayPresentation {
    pub icon: TrayIcon,
    /// `None`: the engine's own tooltip (`engine_status::tray_tooltip`).
    pub tooltip: Option<&'static str>,
    pub click: ClickAction,
    pub menu_attached: bool,
    /// Both mouse buttons act as the click (macOS with the menu detached, C-W10).
    pub both_buttons: bool,
}

pub fn present(view: &AccountView, platform: Platform) -> TrayPresentation {
    todo!("Task 14 step 3")
}

/// macOS: what a click does, or `None` for today's handler (Ready, or a right click while the menu is attached).
pub fn click_action(presentation: &TrayPresentation, button: Button) -> Option<ClickAction> {
    todo!("Task 14 step 3")
}

/// C-I2: the account's tooltip while the b is crossed, else the engine's last one, else "Beebeeb".
pub fn tooltip_text(presentation: &TrayPresentation, engine_tooltip: Option<&str>) -> String {
    todo!("Task 14 step 3")
}

/// C-I2: while the b is crossed, the engine's status events never overwrite the tooltip.
pub fn engine_may_set_tooltip(applied: Option<&TrayPresentation>) -> bool {
    todo!("Task 14 step 3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_view::derive::AccountState;

    fn view(state: AccountState, session: SessionKind, screen: Screen, offline: bool) -> AccountView {
        AccountView { state, session, screen, offline, busy: state == AccountState::Checking, ..AccountView::signed_out(false) }
    }

    fn mac(state: AccountState, session: SessionKind, screen: Screen) -> TrayPresentation {
        present(&view(state, session, screen, false), Platform::Macos)
    }

    #[test]
    fn click_signed_out_opens_sign_in() {
        let p = mac(AccountState::SignedOut, SessionKind::None, Screen::SignIn);
        assert_eq!((p.click, p.menu_attached, p.both_buttons), (ClickAction::OpenAccountWindow, false, true));
    }

    #[test]
    fn click_session_ended_opens_sign_in_again() {
        let p = mac(AccountState::SessionEnded, SessionKind::Ended, Screen::SignInAgain);
        assert_eq!((p.click, p.menu_attached), (ClickAction::OpenAccountWindow, false));
    }

    #[test]
    fn click_update_required_opens_update() {
        for (session, attached) in [(SessionKind::Valid, true), (SessionKind::None, false), (SessionKind::Ended, false)] {
            let p = mac(AccountState::UpdateRequired, session, Screen::Update);
            assert_eq!((p.click, p.menu_attached), (ClickAction::OpenAccountWindow, attached), "{session:?}");
        }
    }

    #[test]
    fn click_locked_with_keychain_key_opens_settings() {
        assert_eq!(mac(AccountState::Locked, SessionKind::Valid, Screen::KeychainUnlock).click, ClickAction::OpenSettingsAccount);
    }

    #[test]
    fn click_locked_without_key_opens_recovery_phrase() {
        assert_eq!(mac(AccountState::Locked, SessionKind::Valid, Screen::RecoveryPhrase).click, ClickAction::OpenAccountWindow);
    }

    #[test]
    fn click_no_plan_opens_choose_a_plan() {
        let p = mac(AccountState::NoPlan, SessionKind::Valid, Screen::ChoosePlan);
        assert_eq!((p.click, p.menu_attached), (ClickAction::OpenAccountWindow, true));
    }

    #[test]
    fn click_checking_opens_busy() {
        assert_eq!(mac(AccountState::Checking, SessionKind::Valid, Screen::Busy).click, ClickAction::OpenAccountWindow);
    }

    #[test]
    fn click_ready_toggles_compact_window() {
        let p = mac(AccountState::Ready, SessionKind::Valid, Screen::Nothing);
        assert_eq!((p.click, p.menu_attached, p.both_buttons), (ClickAction::ToggleCompactWindow, true, false));
        assert_eq!(click_action(&p, Button::Left), None, "today's toggle runs");
    }

    /// C-I1, C-I2, C-W10 for every state.
    #[test]
    fn the_icon_tooltip_and_menu_follow_the_session_and_connectivity() {
        let cases = [
            (AccountState::SignedOut, SessionKind::None, Screen::SignIn, TrayIcon::Crossed, Some(SIGNED_OUT_TOOLTIP), false),
            (AccountState::SessionEnded, SessionKind::Ended, Screen::SignInAgain, TrayIcon::Crossed, Some(SIGN_IN_AGAIN_TOOLTIP), false),
            (AccountState::UpdateRequired, SessionKind::None, Screen::Update, TrayIcon::Crossed, Some(SIGNED_OUT_TOOLTIP), false),
            (AccountState::UpdateRequired, SessionKind::Ended, Screen::Update, TrayIcon::Crossed, Some(SIGN_IN_AGAIN_TOOLTIP), false),
            (AccountState::UpdateRequired, SessionKind::Valid, Screen::Update, TrayIcon::Normal, None, true),
            (AccountState::Locked, SessionKind::Valid, Screen::KeychainUnlock, TrayIcon::Normal, None, true),
            (AccountState::Checking, SessionKind::Valid, Screen::Busy, TrayIcon::Normal, None, true),
            (AccountState::NoPlan, SessionKind::Valid, Screen::ChoosePlan, TrayIcon::Normal, None, true),
            (AccountState::Ready, SessionKind::Valid, Screen::Nothing, TrayIcon::Normal, None, true),
        ];
        for (state, session, screen, icon, tooltip, attached) in cases {
            let p = mac(state, session, screen);
            assert_eq!((p.icon, p.tooltip, p.menu_attached), (icon, tooltip, attached), "{state:?} / {session:?}");
        }
    }

    #[test]
    fn offline_crosses_the_b_and_wins_the_tooltip() {
        for (state, session, screen) in [
            (AccountState::Ready, SessionKind::Valid, Screen::Nothing),
            (AccountState::SignedOut, SessionKind::None, Screen::SignIn),
            (AccountState::SessionEnded, SessionKind::Ended, Screen::SignInAgain),
        ] {
            let p = present(&view(state, session, screen, true), Platform::Macos);
            assert_eq!((p.icon, p.tooltip), (TrayIcon::Crossed, Some(OFFLINE_TOOLTIP)), "{state:?}");
        }
    }

    #[test]
    fn windows_and_linux_get_the_crossed_icon_and_keep_their_clicks_and_menu() {
        for platform in [Platform::Windows, Platform::Linux] {
            let p = present(&view(AccountState::SignedOut, SessionKind::None, Screen::SignIn, false), platform);
            assert_eq!((p.icon, p.tooltip), (TrayIcon::Crossed, Some(SIGNED_OUT_TOOLTIP)));
            assert_eq!((p.click, p.menu_attached, p.both_buttons), (ClickAction::Unchanged, true, false));
        }
    }

    #[test]
    fn both_buttons_open_the_window_only_while_the_menu_is_detached() {
        let signed_out = mac(AccountState::SignedOut, SessionKind::None, Screen::SignIn);
        assert_eq!(click_action(&signed_out, Button::Right), Some(ClickAction::OpenAccountWindow));
        let no_plan = mac(AccountState::NoPlan, SessionKind::Valid, Screen::ChoosePlan);
        assert_eq!(click_action(&no_plan, Button::Left), Some(ClickAction::OpenAccountWindow));
        assert_eq!(click_action(&no_plan, Button::Right), None, "with a session the right click keeps the menu (C-W11)");
    }

    #[test]
    fn leaving_the_crossed_b_reapplies_the_last_engine_tooltip_at_once() {
        let crossed = mac(AccountState::SignedOut, SessionKind::None, Screen::SignIn);
        let normal = mac(AccountState::Ready, SessionKind::Valid, Screen::Nothing);
        assert_eq!(tooltip_text(&crossed, Some("Beebeeb · Synced")), SIGNED_OUT_TOOLTIP);
        assert_eq!(tooltip_text(&normal, Some("Beebeeb · Synced")), "Beebeeb · Synced");
        assert_eq!(tooltip_text(&normal, None), "Beebeeb");
        assert!(!engine_may_set_tooltip(Some(&crossed)));
        assert!(engine_may_set_tooltip(Some(&normal)));
        assert!(engine_may_set_tooltip(None));
    }
}
```

Add `mod tray_presentation;` to `lib.rs`. Run with filter `tray_presentation`. Expected: `test result: FAILED. 0 passed; 13 failed`.

- [ ] **Step 3: Implement**

```rust
pub fn present(view: &AccountView, platform: Platform) -> TrayPresentation {
    let tooltip = if view.offline {
        Some(OFFLINE_TOOLTIP)
    } else {
        match view.session {
            SessionKind::None => Some(SIGNED_OUT_TOOLTIP),
            SessionKind::Ended => Some(SIGN_IN_AGAIN_TOOLTIP),
            SessionKind::Valid => None,
        }
    };
    let icon = if tooltip.is_some() { TrayIcon::Crossed } else { TrayIcon::Normal };
    if platform != Platform::Macos {
        return TrayPresentation { icon, tooltip, click: ClickAction::Unchanged, menu_attached: true, both_buttons: false };
    }
    let menu_attached = view.session == SessionKind::Valid;
    let click = match view.screen {
        Screen::Nothing => ClickAction::ToggleCompactWindow,
        Screen::KeychainUnlock => ClickAction::OpenSettingsAccount,
        _ => ClickAction::OpenAccountWindow,
    };
    TrayPresentation { icon, tooltip, click, menu_attached, both_buttons: !menu_attached }
}

pub fn click_action(presentation: &TrayPresentation, button: Button) -> Option<ClickAction> {
    match (presentation.click, button) {
        (ClickAction::Unchanged | ClickAction::ToggleCompactWindow, _) => None,
        (action, Button::Left) => Some(action),
        (action, Button::Right) if presentation.both_buttons => Some(action),
        (_, Button::Right) => None,
    }
}

pub fn tooltip_text(presentation: &TrayPresentation, engine_tooltip: Option<&str>) -> String {
    presentation.tooltip.or(engine_tooltip).unwrap_or("Beebeeb").to_string()
}

pub fn engine_may_set_tooltip(applied: Option<&TrayPresentation>) -> bool {
    applied.is_none_or(|presentation| presentation.icon == TrayIcon::Normal)
}
```

Run with filter `tray_presentation`. Expected: `test result: ok. 13 passed; 0 failed`. Mutation: make `menu_attached` always `true`; expect `click_signed_out_opens_sign_in`, `click_update_required_opens_update` and `both_buttons_…`; revert. Make the offline check come after the session match; expect `offline_crosses_the_b_and_wins_the_tooltip`; revert.

- [ ] **Step 4: Enable `image-ico` and prove the lockfile adds no package (C-I4)**

`src-tauri/Cargo.toml`:

```toml
tauri = { version = "2", features = ["tray-icon", "image-png", "image-ico"] }
```

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/t14-ico-check.log 2>&1; echo "rc=$?"
/usr/bin/grep -c '^\[\[package\]\]' Cargo.lock > $EVID/t14-lock-after.txt; shasum -a 256 Cargo.lock >> $EVID/t14-lock-after.txt
diff $EVID/t14-lock-before.txt $EVID/t14-lock-after.txt > $EVID/t14-lock-diff.log; echo "diff rc=$?"
git diff --stat -- Cargo.lock
```

Expected: `cargo check --locked` `rc=0` (it refuses to run if the lockfile would change), `diff rc=0`, and no `Cargo.lock` line in `git diff --stat`. If `--locked` fails because the lockfile must change: **stop** and report BLOCKED (OP-6 ruled no new package); do not run `cargo update`.

- [ ] **Step 5: Embed the icons and check them**

In `lib.rs`, near `setup_tray`:

```rust
/// Spec 2026-10-07 C-I3/C-I4: the icons the tray swaps between. macOS: template images (black and alpha), only the 44×44
/// files, as the config uses for today's icon. Windows and Linux: today's colour icon and its crossed `.ico`.
const TRAY_TEMPLATE: &[u8] = include_bytes!("../icons/tray-template@2x.png");
const TRAY_TEMPLATE_DISCONNECTED: &[u8] = include_bytes!("../icons/tray-template-disconnected@2x.png");
const TRAY_COLOUR: &[u8] = include_bytes!("../icons/icon.png");
const TRAY_COLOUR_DISCONNECTED: &[u8] = include_bytes!("../icons/tray-disconnected.ico");

fn tray_icon_bytes(icon: tray_presentation::TrayIcon) -> &'static [u8] {
    use tray_presentation::TrayIcon;
    match (cfg!(target_os = "macos"), icon) {
        (true, TrayIcon::Normal) => TRAY_TEMPLATE,
        (true, TrayIcon::Crossed) => TRAY_TEMPLATE_DISCONNECTED,
        (false, TrayIcon::Normal) => TRAY_COLOUR,
        (false, TrayIcon::Crossed) => TRAY_COLOUR_DISCONNECTED,
    }
}
```

Tests (any `lib.rs` test module):

```rust
    /// C-I3: the crossed template is today's b, byte for byte outside the slash's knockout band, black and alpha only.
    #[test]
    fn the_disconnected_template_is_todays_b_with_the_variant_b_slash() {
        let old = tauri::image::Image::from_bytes(TRAY_TEMPLATE).unwrap();
        let new = tauri::image::Image::from_bytes(TRAY_TEMPLATE_DISCONNECTED).unwrap();
        assert_eq!((new.width(), new.height()), (44, 44));
        let mut compared = 0;
        for (index, (before, after)) in old.rgba().chunks(4).zip(new.rgba().chunks(4)).enumerate() {
            let (x, y) = ((index % 44) as f64 + 0.5, (index / 44) as f64 + 0.5);
            if after[3] != 0 {
                assert_eq!(&after[..3], &[0, 0, 0], "black and alpha only, at {x},{y}");
            }
            if (x + y - 44.0).abs() / std::f64::consts::SQRT_2 > 5.6 {
                assert_eq!(before, after, "outside the slash, at {x},{y}");
                compared += 1;
            }
        }
        assert!(compared > 1000, "{compared}");
    }

    /// C-I4: the colour crossed icon decodes (tauri's `image-ico`).
    #[test]
    fn the_colour_disconnected_icon_decodes() {
        let icon = tauri::image::Image::from_bytes(TRAY_COLOUR_DISCONNECTED).unwrap();
        assert!(icon.width() >= 16 && icon.width() == icon.height());
    }
```

Run them. Expected: both pass. Mutation: point `TRAY_TEMPLATE_DISCONNECTED` at `tray-template.png` (22×22); expect the size assertion to fail; revert.

- [ ] **Step 6: Apply the presentation at runtime**

In `lib.rs`:

```rust
/// What the tray shows now, and the engine's last tooltip (re-applied the moment the b is normal again).
struct TrayState {
    applied: Option<tray_presentation::TrayPresentation>,
    engine_tooltip: Option<String>,
}

static TRAY_STATE: std::sync::Mutex<TrayState> = std::sync::Mutex::new(TrayState { applied: None, engine_tooltip: None });

/// Spec 2026-10-07 C-I5: swap the icon, the tooltip and (macOS) the menu through the config-built tray `"tray"`, only
/// when the presentation changes, never on an engine tick.
pub(crate) fn apply_tray_presentation(app: &tauri::AppHandle, view: &account_view::derive::AccountView) {
    let presentation = tray_presentation::present(view, surfaces::policy::Platform::current());
    let mut tray_state = TRAY_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if tray_state.applied == Some(presentation) {
        return;
    }
    let previous = tray_state.applied.replace(presentation);
    let Some(tray) = app.tray_by_id("tray") else {
        return;
    };
    if previous.map(|p| p.icon) != Some(presentation.icon) {
        match tauri::image::Image::from_bytes(tray_icon_bytes(presentation.icon)) {
            Ok(image) => {
                if let Err(error) = tray.set_icon(Some(image)) {
                    tracing::warn!(%error, "tray: could not set the icon");
                }
                #[cfg(target_os = "macos")]
                let _ = tray.set_icon_as_template(true);
            }
            Err(error) => tracing::warn!(%error, "tray: could not decode the icon"),
        }
    }
    let tooltip = tray_presentation::tooltip_text(&presentation, tray_state.engine_tooltip.as_deref());
    if let Err(error) = tray.set_tooltip(Some(&tooltip)) {
        tracing::warn!(%error, "tray: could not set the tooltip");
    }
    #[cfg(target_os = "macos")]
    if previous.map(|p| p.menu_attached) != Some(presentation.menu_attached) {
        let menu = if presentation.menu_attached {
            build_tray_menu(app, app.autolaunch().is_enabled().unwrap_or(false)).ok()
        } else {
            None
        };
        if let Err(error) = tray.set_menu(menu) {
            tracing::warn!(%error, "tray: could not attach or detach the menu");
        }
    }
}
```

(If `build_tray_menu` takes `&tauri::App`, change its parameter to a generic `impl Manager<R>` the way its `on_menu_event` caller already uses it with an `AppHandle`.)

In `attach_tray_status_listener`, replace the final `set_tooltip` block with:

```rust
        let tooltip = engine_status::tray_tooltip(&payload);
        // Spec 2026-10-07 C-I2: remember the engine's tooltip; while the b is crossed, the account's stands.
        let may_set = {
            let mut tray_state = TRAY_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            tray_state.engine_tooltip = Some(tooltip.clone());
            tray_presentation::engine_may_set_tooltip(tray_state.applied.as_ref())
        };
        if may_set
            && let Some(tray) = app_for_listener.tray_by_id("tray")
            && let Err(e) = tray.set_tooltip(Some(&tooltip))
        {
            tracing::warn!(error = %e, "failed to update tray tooltip");
        }
```

In `app_ports.rs`'s `AppPublisher::publish`, after the emit (plan review C1: every published view, the launch view and the sign-out/switch clear included, reaches the icon from here, and only from here):

```rust
        // The publish lock is held: post the icon change to the main thread instead of waiting for it. The main thread
        // runs the posts in publish order.
        let app = self.app.clone();
        let view = view.clone();
        if let Err(error) = self.app.run_on_main_thread(move || crate::apply_tray_presentation(&app, &view)) {
            tracing::warn!(%error, "tray: could not post the presentation");
        }
```

No call in `setup()`: `driver::start` publishes the launch view (Task 10), and the account-view block of Task 11 runs after `setup_tray`, so the tray exists when the post runs.

The click (macOS). At the top of the closure given to `tray.on_tray_icon_event`:

```rust
        // Spec 2026-10-07 C-W3/C-W10/C-W11 (macOS): any state but Ready opens its window; with the menu detached both
        // buttons do. Ready falls through to today's toggle, which lands on the Finder location view (plan Task 20).
        #[cfg(target_os = "macos")]
        {
            if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = &event {
                let app = tray.app_handle();
                let pressed = match button {
                    MouseButton::Left => Some(tray_presentation::Button::Left),
                    MouseButton::Right => Some(tray_presentation::Button::Right),
                    _ => None,
                };
                if let Some(view) = app.state::<AppState>().account_view.get().map(|handle| handle.state())
                    && let Some(pressed) = pressed
                {
                    let presentation = tray_presentation::present(&view, surfaces::policy::Platform::Macos);
                    match tray_presentation::click_action(&presentation, pressed) {
                        Some(tray_presentation::ClickAction::OpenAccountWindow) => {
                            open_surface(app, surfaces::policy::StartupSurface::AccountWindow, &view);
                            return;
                        }
                        Some(tray_presentation::ClickAction::OpenSettingsAccount) => {
                            show_macos_settings_window_on_account(app);
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }
```

(`event` is taken by reference here so today's `if let TrayIconEvent::Click { button: MouseButton::Left, … } = event` below still matches it. The `#[cfg]` sits on a block, not on the `if let` statement.)

The crossed b after a live sign-out and a switch (plan review C1, lead ruling): in `account_view/driver.rs`'s tests, at the end of `a_live_sign_out_publishes_signed_out` and of `a_switch_publishes_signed_out_and_then_the_next_account`, add (with `last` in the first and `cleared` in the second):

```rust
        assert_eq!(
            crate::tray_presentation::present(&last, Platform::Macos).icon,
            crate::tray_presentation::TrayIcon::Crossed,
            "the published Signed out view shows the crossed b"
        );
```

These two read the views the publisher received, and `AppPublisher::publish` is the one place that applies the icon (pinned in Step 7).

- [ ] **Step 7: Source tests for the runtime wiring**

In `finder_setup_command_tests`:

```rust
    /// C-I5: the swap goes through the config-built tray, only on a change; the engine never overwrites a crossed b.
    #[test]
    fn the_tray_swaps_only_on_a_change_and_the_engine_respects_the_crossed_b() {
        let production = production_source();
        let apply = body_between(&production, "fn apply_tray_presentation(", "\n}\n");
        let unchanged = apply.find("if tray_state.applied == Some(presentation) {").expect("only on a change");
        let swap = apply.find("tray.set_icon(").expect("the swap");
        assert!(unchanged < swap);
        assert!(apply.contains("set_icon_as_template(true)") && apply.contains("tray_by_id(\"tray\")"));
        let listener = body_between(&production, "fn attach_tray_status_listener", "\n}\n");
        assert!(listener.contains("engine_may_set_tooltip("));
        let ports = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/account_view/app_ports.rs")).unwrap();
        let publish = &ports[ports.find("impl Publisher for AppPublisher").expect("the app's publisher")..];
        let publish = &publish[..publish.find("\n}\n").expect("its end")];
        assert!(publish.contains("run_on_main_thread(") && publish.contains("apply_tray_presentation("), "every published view reaches the icon, posted:\n{publish}");
        assert_eq!(production.matches("apply_tray_presentation(").count(), 1, "only its definition in lib.rs; the publisher is the one caller");
    }

    /// C-W10: with no valid session the click opens the window from either button, before today's handler runs.
    #[test]
    fn the_macos_click_follows_the_account_view_before_todays_toggle() {
        let production = production_source();
        let tray = body_between(&production, "fn setup_tray(", "\n}\n");
        let account = tray.find("tray_presentation::click_action(").expect("the account click");
        let today = tray.find("button: MouseButton::Left,").expect("today's left-click handler");
        assert!(account < today);
        assert!(tray.contains("MouseButton::Right => Some(tray_presentation::Button::Right)"));
    }
```

Run with the filter `-- the_tray_swaps_only_on_a_change_and_the_engine_respects_the_crossed_b the_macos_click_follows_the_account_view_before_todays_toggle a_live_sign_out_publishes_signed_out a_switch_publishes_signed_out_and_then_the_next_account`. Expected: `test result: ok. 4 passed; 0 failed`.

Mutation-check (paste each failure, revert, paste the green):
1. Delete `if tray_state.applied == Some(presentation) { return; }`. Expected: `the_tray_swaps_only_on_a_change_and_the_engine_respects_the_crossed_b`.
2. Delete the `run_on_main_thread(…)` statement from `AppPublisher::publish`. Expected: the same test ("every published view reaches the icon, posted").
3. Add `apply_tray_presentation(app.handle(), &launch.view);` back into `setup()`. Expected: the same test ("the publisher is the one caller").
4. Delete the macOS `#[cfg]` click block. Expected: `the_macos_click_follows_the_account_view_before_todays_toggle`.
5. In `present`, give `SessionKind::None` the `Normal` icon. Expected: `a_live_sign_out_publishes_signed_out` and `a_switch_publishes_signed_out_and_then_the_next_account` ("the published Signed out view shows the crossed b"), with Step 2's `the_icon_tooltip_and_menu_follow_the_session_and_connectivity`.

- [ ] **Step 8: Run everything, check clippy against the baseline, commit**

```bash
cd $WT/src-tauri && SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/t14-all.log 2>&1; echo "rc=$?"
rm -rf "$SCRATCH"; /usr/bin/grep "test result:" $EVID/t14-all.log
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t14-clippy.log 2>&1; echo "rc=$?"
/usr/bin/grep -c '^warning' $EVID/t14-clippy.log; cat $EVID/t0-baseline-clippy-warnings.txt
cd $WT && git add src-tauri/src/tray_presentation.rs src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/src/account_view/app_ports.rs src-tauri/src/account_view/driver.rs
git commit -m "desktop: show a crossed menu-bar icon while disconnected and open the account window from it" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src-tauri/src/tray_presentation.rs src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/src/account_view/app_ports.rs src-tauri/src/account_view/driver.rs
git show --stat HEAD
```

Expected: every binary `0 failed`; on macOS the lib is `B_lib` + 181 (+ 1 if Task 12's M5 steps ran); the clippy warning count is not above the baseline (list any new warning in a file this plan created and fix it before the commit).

---

## Task 15: The frontend contract — the view, its copy and its links

**Lane T.** No UI yet. Everything later TS tasks import. The commands are mocked through the harness's `__TAURI_INTERNALS__` (the same technique as `tests/finderSetup.test.ts`).

**Files:**
- Create: `src/accountView.ts`, `src/accountViewCopy.ts`, `src/accountLinks.ts`
- Test: `tests/accountView.test.ts`, `tests/accountViewCopy.test.ts`, `tests/accountLinks.test.ts`

**Interfaces:**
- Consumes: the Rust JSON contract (Task 5's three examples, the notice JSON of Task 4), the commands `account_view_state`, `account_view_retry`, `account_view_plan_opened` (Task 11), the event `account-view-changed` (Task 10); spec A's `command`, `CommandResult`, `openUrl` (`src/desktopApi.ts`).
- Produces:
  - `src/accountView.ts`: `ACCOUNT_STATES`, `SESSION_KINDS`, `ACCOUNT_SCREENS`, `NOTICE_KINDS`, `NOTICE_LINKS` (const arrays); types `AccountState`, `SessionKind`, `AccountScreen`, `NoticeKind`, `NoticeLink`, `AccountLinks`, `AccountNotice`, `AccountView`, `AccountViewLoad`; `ACCOUNT_VIEW_CHANGED_EVENT`; `parseAccountView(value: unknown): AccountView | null`; `loadAccountView(): Promise<CommandResult<AccountView>>`; `subscribeAccountView(onView, options?: { listen? }): () => void`; `useAccountView(): AccountViewLoad`; `retryAccountView()`, `accountViewPlanOpened()`
  - `src/accountViewCopy.ts`: every §7 string (names below), `NOTICE_LINK_LABEL`, `noticeLine(notice: AccountNotice): string | null`
  - `src/accountLinks.ts`: `BUILT_IN_LINKS`, `type AccountDestination`, `linkFor(view, destination): string`, `type LinkOpen`, `openAccountLink(url, open?): Promise<LinkOpen>`, `copyAccountLink(url, write?): Promise<boolean>`

- [ ] **Step 1: Write the failing tests**

`tests/accountView.test.ts`:

```ts
/**
 * The frontend end of the account view (spec 2026-10-07 §13). The three examples are copies of Rust's
 * `src-tauri/tests/fixtures/account-view/*.json` (Task 5); after Lane T rebases onto Lane R, Task 21 adds a test that
 * reads those files directly, so a rename on either side fails here and not in a window.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import {
  ACCOUNT_VIEW_CHANGED_EVENT,
  accountViewPlanOpened,
  loadAccountView,
  parseAccountView,
  retryAccountView,
  subscribeAccountView,
  type AccountView,
} from '../src/accountView'

const LINKS = {
  create_account: 'https://app.beebeeb.io/signup',
  billing: 'https://app.beebeeb.io/billing',
  support: 'https://beebeeb.io/support',
  step: 'https://app.beebeeb.io/billing',
}
const SIGNED_OUT = { state: 'signed_out', session: 'none', offline: false, screen: 'sign_in', busy: false, links: LINKS, notice: null }
const READY_TRIAL = {
  state: 'ready', session: 'valid', offline: false, screen: 'nothing', busy: false, links: LINKS,
  notice: { kind: 'trial', server_line: null, trial_ends: '18 Oct', read_only_since: null, data_deletion_at: null, link: 'manage_plan', url: 'https://app.beebeeb.io/billing' },
}
const NO_PLAN_OFFLINE = { state: 'no_plan', session: 'valid', offline: true, screen: 'choose_plan', busy: false, links: LINKS, notice: null }

const previousWindow = (globalThis as any).window
afterEach(() => { (globalThis as any).window = previousWindow })

function backend(handlers: Record<string, (args: any) => unknown>) {
  const calls: Array<{ name: string; args: any }> = []
  ;(globalThis as any).window = {
    __TAURI_INTERNALS__: {
      invoke: async (name: string, args: any) => {
        calls.push({ name, args })
        const handler = handlers[name]
        if (!handler) throw new Error(`unscripted command ${name}`)
        return handler(args)
      },
    },
  }
  return calls
}

describe('parseAccountView', () => {
  test('reads the three Rust examples', () => {
    for (const example of [SIGNED_OUT, READY_TRIAL, NO_PLAN_OFFLINE]) {
      expect(parseAccountView(example)).toEqual(example as AccountView)
    }
  })

  test('refuses anything that does not match the contract', () => {
    const bad: unknown[] = [
      null,
      'ready',
      { ...SIGNED_OUT, state: 'future_state' },
      { ...SIGNED_OUT, session: 'maybe' },
      { ...SIGNED_OUT, screen: 'popover' },
      { ...SIGNED_OUT, offline: 'no' },
      { ...SIGNED_OUT, busy: 1 },
      { ...SIGNED_OUT, links: { ...LINKS, billing: '' } },
      { ...SIGNED_OUT, links: { create_account: LINKS.create_account } },
      { ...READY_TRIAL, notice: { ...READY_TRIAL.notice, kind: 'future_kind' } },
      { ...READY_TRIAL, notice: { ...READY_TRIAL.notice, link: 'somewhere' } },
      { ...READY_TRIAL, notice: { ...READY_TRIAL.notice, trial_ends: 18 } },
    ]
    for (const value of bad) expect({ value, parsed: parseAccountView(value) }).toEqual({ value, parsed: null })
  })
})

describe('the commands and the event', () => {
  test('loadAccountView reads account_view_state and fails on an unexpected shape', async () => {
    backend({ account_view_state: () => READY_TRIAL })
    expect(await loadAccountView()).toEqual({ ok: true, value: READY_TRIAL as AccountView })
    backend({ account_view_state: () => ({ state: 'ready' }) })
    const result = await loadAccountView()
    expect(result.ok).toBe(false)
  })

  test('retry and plan opened send their commands', async () => {
    const calls = backend({ account_view_retry: () => null, account_view_plan_opened: () => null })
    await retryAccountView()
    await accountViewPlanOpened()
    expect(calls.map((c) => c.name)).toEqual(['account_view_retry', 'account_view_plan_opened'])
  })

  test('subscribeAccountView passes parsed views on, drops bad ones, and stops', async () => {
    let handler: ((event: { payload: unknown }) => void) | null = null
    let unlistened = 0
    const listen = (async (name: string, h: (event: { payload: unknown }) => void) => {
      expect(name).toBe(ACCOUNT_VIEW_CHANGED_EVENT)
      handler = h
      return () => { unlistened++ }
    }) as any
    const seen: AccountView[] = []
    const stop = subscribeAccountView((view) => seen.push(view), { listen })
    await new Promise((resolve) => setTimeout(resolve, 0))
    handler!({ payload: NO_PLAN_OFFLINE })
    handler!({ payload: { state: 'nonsense' } })
    expect(seen).toEqual([NO_PLAN_OFFLINE as AccountView])
    stop()
    handler!({ payload: SIGNED_OUT })
    expect(seen).toHaveLength(1)
    expect(unlistened).toBe(1)
  })
})
```

`tests/accountViewCopy.test.ts`:

```ts
/**
 * Every word the account view's screens say (spec 2026-10-07 §7), pinned to the approved copy, and the notice line for
 * every row and read-only variant of §7.5.
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import * as copy from '../src/accountViewCopy'
import type { AccountNotice } from '../src/accountView'

const notice = (fields: Partial<AccountNotice>): AccountNotice => ({
  kind: 'trial', server_line: null, trial_ends: null, read_only_since: null, data_deletion_at: null, link: null, url: null, ...fields,
})

describe('the approved copy', () => {
  test('every screen string is exactly as approved', () => {
    expect(copy.SIGN_IN_TITLE).toBe('Sign in to Beebeeb')
    expect(copy.SIGN_IN_WITH_BROWSER).toBe('Sign in with browser')
    expect(copy.USE_EMAIL_AND_PASSWORD).toBe('Use email and password')
    expect(`${copy.CREATE_ACCOUNT_PROMPT} ${copy.CREATE_ACCOUNT_LINK}`).toBe('New to Beebeeb? Create an account on beebeeb.io')
    expect(copy.QUIT_BEEBEEB).toBe('Quit Beebeeb')
    expect(copy.OFFLINE_LINE).toBe('Can’t reach Beebeeb. Check your connection.')
    expect(copy.TRY_AGAIN).toBe('Try again')
    expect(copy.SESSION_ENDED_LINE).toBe('Sync is paused until you sign in again.')
    expect(copy.SIGN_IN_AGAIN).toBe('Sign in again')
    expect(copy.UPDATE_TITLE).toBe('Update Beebeeb')
    expect(copy.UPDATE_BODY).toBe('This version of Beebeeb is too old to connect. Update to continue.')
    expect(copy.UPDATE_NOW).toBe('Update now')
    expect(copy.CHOOSE_PLAN_TITLE).toBe('Choose a plan')
    expect(copy.CHOOSE_PLAN_BODY).toBe('Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.')
    expect(copy.CHOOSE_PLAN_BUTTON).toBe('Choose a plan on beebeeb.io')
    expect(copy.CHOOSE_PLAN_WAITING).toBe('Waiting for your plan. This window updates by itself.')
    expect(copy.SIGN_OUT).toBe('Sign out')
    expect(copy.COPY_LINK).toBe('Copy link')
    expect(copy.NOTICE_LINK_LABEL).toEqual({
      manage_plan: 'Manage plan', choose_plan: 'Choose a plan', contact_support: 'Contact support', update_payment_details: 'Update payment details',
    })
  })

  test('the session-ended words are spec A’s, unchanged', () => {
    const banner = readFileSync(new URL('../src/AuthExpiredBanner.tsx', import.meta.url), 'utf8')
    expect(banner).toContain(`'${copy.SESSION_ENDED_LINE}'`)
    expect(banner).toContain(`'${copy.SIGN_IN_AGAIN}'`)
  })

  test('no screen string uses a straight apostrophe', () => {
    for (const [name, value] of Object.entries(copy)) {
      if (typeof value === 'string') expect({ name, straight: value.includes("'") }).toEqual({ name, straight: false })
    }
  })
})

describe('noticeLine (§7.5)', () => {
  test('a trial names its end date', () => {
    expect(copy.noticeLine(notice({ kind: 'trial', trial_ends: '18 Oct' }))).toBe('Free trial until 18 Oct.')
  })

  test('every read-only variant drops the clause whose date is missing, never inventing one', () => {
    const line = (read_only_since: string | null, data_deletion_at: string | null) =>
      copy.noticeLine(notice({ kind: 'read_only', read_only_since, data_deletion_at }))
    expect(line('18 Oct', '1 Nov')).toBe('Read-only since 18 Oct. Files are deleted on 1 Nov unless you choose a plan.')
    expect(line(null, '1 Dec')).toBe('Read-only. Files are deleted on 1 Dec unless you choose a plan.')
    expect(line('6 Oct', null)).toBe('Read-only since 6 Oct. Choose a plan to upload again.')
    expect(line(null, null)).toBe('Read-only. Choose a plan to upload again.')
  })

  test('the other rows', () => {
    expect(copy.noticeLine(notice({ kind: 'read_only_other' }))).toBe('Read-only.')
    expect(copy.noticeLine(notice({ kind: 'frozen' }))).toBe('This account is frozen.')
    expect(copy.noticeLine(notice({ kind: 'payment_failed' }))).toBe('Payment failed. Update your payment details on beebeeb.io.')
  })

  test('the server’s words win over every built-in line, verbatim', () => {
    for (const kind of ['trial', 'read_only', 'read_only_other', 'frozen', 'payment_failed'] as const) {
      expect(copy.noticeLine(notice({ kind, server_line: 'Words from the server.', trial_ends: '18 Oct' }))).toBe('Words from the server.')
    }
  })

  test('a trial with neither a date nor server words has no line', () => {
    expect(copy.noticeLine(notice({ kind: 'trial' }))).toBeNull()
  })
})
```

`tests/accountLinks.test.ts`:

```ts
/** Spec 2026-10-07 §5.1: the built-in addresses, and opening exactly the address the view names. */
import { describe, expect, test } from 'bun:test'
import { BUILT_IN_LINKS, copyAccountLink, linkFor, openAccountLink } from '../src/accountLinks'
import type { AccountView } from '../src/accountView'

const view = {
  state: 'ready', session: 'valid', offline: false, screen: 'nothing', busy: false, notice: null,
  links: { create_account: 'https://app.beebeeb.io/signup?from=server', billing: 'https://app.beebeeb.io/billing?from=server', support: 'https://beebeeb.io/support?from=server', step: 'https://app.beebeeb.io/step?from=server' },
} as AccountView

describe('accountLinks', () => {
  test('the built-in addresses are §5.1’s', () => {
    expect(BUILT_IN_LINKS).toEqual({ create_account: 'https://app.beebeeb.io/signup', billing: 'https://app.beebeeb.io/billing', support: 'https://beebeeb.io/support' })
  })

  test('linkFor reads the view, and the built-in addresses only before it has loaded', () => {
    for (const destination of ['create_account', 'billing', 'support', 'step'] as const) {
      expect(linkFor(view, destination)).toBe(view.links[destination])
    }
    expect(linkFor(null, 'create_account')).toBe(BUILT_IN_LINKS.create_account)
    expect(linkFor(undefined, 'support')).toBe(BUILT_IN_LINKS.support)
    expect(linkFor(null, 'step')).toBe(BUILT_IN_LINKS.billing)
  })

  test('openAccountLink opens exactly the given address and reports a failure with it', async () => {
    const opened: string[] = []
    expect(await openAccountLink(view.links.step, async (url) => { opened.push(url); return { ok: true, value: undefined } })).toEqual({ opened: true })
    expect(opened).toEqual([view.links.step])
    expect(await openAccountLink('https://app.beebeeb.io/billing', async () => ({ ok: false, reason: 'no browser', unsupported: false })))
      .toEqual({ opened: false, url: 'https://app.beebeeb.io/billing' })
  })

  test('copyAccountLink writes the address and says when it could not', async () => {
    const written: string[] = []
    expect(await copyAccountLink('https://app.beebeeb.io/billing', async (text) => { written.push(text) })).toBe(true)
    expect(written).toEqual(['https://app.beebeeb.io/billing'])
    expect(await copyAccountLink('x', async () => { throw new Error('denied') })).toBe(false)
  })
})
```

Run: `bun test tests/accountView.test.ts tests/accountViewCopy.test.ts tests/accountLinks.test.ts > $EVID/t15-red.log 2>&1; echo "rc=$?"`. Expected: `rc=1`, module-not-found errors for the three new files. Paste them.

- [ ] **Step 2: `src/accountView.ts`**

```ts
/**
 * The frontend end of the account view (spec 2026-10-07 §13): the shape Rust's `AccountView` serialises, its command
 * and its event. Every window renders this and decides nothing itself. Strict: a value that does not match the
 * contract is `null`, and the caller shows its failed state, never a guess.
 */
import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, type CommandResult } from './desktopApi'

export const ACCOUNT_STATES = ['checking', 'signed_out', 'session_ended', 'update_required', 'locked', 'no_plan', 'ready'] as const
export type AccountState = (typeof ACCOUNT_STATES)[number]
export const SESSION_KINDS = ['none', 'valid', 'ended'] as const
export type SessionKind = (typeof SESSION_KINDS)[number]
export const ACCOUNT_SCREENS = ['sign_in', 'sign_in_again', 'update', 'recovery_phrase', 'keychain_unlock', 'choose_plan', 'busy', 'nothing'] as const
export type AccountScreen = (typeof ACCOUNT_SCREENS)[number]
export const NOTICE_KINDS = ['trial', 'read_only', 'read_only_other', 'frozen', 'payment_failed'] as const
export type NoticeKind = (typeof NOTICE_KINDS)[number]
export const NOTICE_LINKS = ['manage_plan', 'choose_plan', 'contact_support', 'update_payment_details'] as const
export type NoticeLink = (typeof NOTICE_LINKS)[number]

export interface AccountLinks {
  create_account: string
  billing: string
  support: string
  step: string
}

export interface AccountNotice {
  kind: NoticeKind
  server_line: string | null
  trial_ends: string | null
  read_only_since: string | null
  data_deletion_at: string | null
  link: NoticeLink | null
  url: string | null
}

export interface AccountView {
  state: AccountState
  session: SessionKind
  offline: boolean
  screen: AccountScreen
  busy: boolean
  links: AccountLinks
  notice: AccountNotice | null
}

export type AccountViewLoad = { status: 'loading' } | { status: 'failed' } | { status: 'ready'; view: AccountView }

export const ACCOUNT_VIEW_CHANGED_EVENT = 'account-view-changed'

const isObject = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null && !Array.isArray(x)
const oneOf = <T extends string>(values: readonly T[], x: unknown): x is T => typeof x === 'string' && (values as readonly string[]).includes(x)
const isAddress = (x: unknown): x is string => typeof x === 'string' && x.length > 0
const stringOrNull = (x: unknown): x is string | null => x === null || typeof x === 'string'

function parseNotice(value: unknown): AccountNotice | null | undefined {
  if (value === null) return null
  if (!isObject(value)) return undefined
  const { kind, server_line, trial_ends, read_only_since, data_deletion_at, link, url } = value
  if (!oneOf(NOTICE_KINDS, kind)) return undefined
  if (![server_line, trial_ends, read_only_since, data_deletion_at, url].every(stringOrNull)) return undefined
  if (link !== null && !oneOf(NOTICE_LINKS, link)) return undefined
  return { kind, server_line, trial_ends, read_only_since, data_deletion_at, link, url } as AccountNotice
}

export function parseAccountView(value: unknown): AccountView | null {
  if (!isObject(value)) return null
  const { state, session, offline, screen, busy, links, notice } = value
  if (!oneOf(ACCOUNT_STATES, state) || !oneOf(SESSION_KINDS, session) || !oneOf(ACCOUNT_SCREENS, screen)) return null
  if (typeof offline !== 'boolean' || typeof busy !== 'boolean' || !isObject(links)) return null
  const { create_account, billing, support, step } = links
  if (!isAddress(create_account) || !isAddress(billing) || !isAddress(support) || !isAddress(step)) return null
  const parsed = parseNotice(notice)
  if (parsed === undefined) return null
  return { state, session, offline, screen, busy, links: { create_account, billing, support, step }, notice: parsed }
}

export async function loadAccountView(): Promise<CommandResult<AccountView>> {
  const result = await command<unknown>('account_view_state')
  if (!result.ok) return result
  const view = parseAccountView(result.value)
  return view ? { ok: true, value: view } : { ok: false, reason: 'The account state had an unexpected shape.', unsupported: false }
}

export function subscribeAccountView(onView: (view: AccountView) => void, options: { listen?: typeof listen } = {}): () => void {
  const listenTo = options.listen ?? listen
  let stopped = false
  let unlisten: (() => void) | null = null
  listenTo<unknown>(ACCOUNT_VIEW_CHANGED_EVENT, (event) => {
    const view = parseAccountView(event.payload)
    if (view && !stopped) onView(view)
  })
    .then((stop) => {
      if (stopped) stop()
      else unlisten = stop
    })
    .catch(() => {})
  return () => {
    stopped = true
    unlisten?.()
  }
}

/** The view, kept current by the event. A view pushed while the first read is in flight is newer, so it wins. */
export function useAccountView(): AccountViewLoad {
  const [load, setLoad] = useState<AccountViewLoad>({ status: 'loading' })
  useEffect(() => {
    let cancelled = false
    let pushed = false
    const stop = subscribeAccountView((view) => {
      pushed = true
      if (!cancelled) setLoad({ status: 'ready', view })
    })
    void loadAccountView().then((result) => {
      if (cancelled || pushed) return
      setLoad(result.ok ? { status: 'ready', view: result.value } : { status: 'failed' })
    })
    return () => {
      cancelled = true
      stop()
    }
  }, [])
  return load
}

export function retryAccountView(): Promise<CommandResult<void>> {
  return command<void>('account_view_retry')
}

export function accountViewPlanOpened(): Promise<CommandResult<void>> {
  return command<void>('account_view_plan_opened')
}
```

- [ ] **Step 3: `src/accountViewCopy.ts`**

```ts
/**
 * Every word the account view's screens say (spec 2026-10-07 §7), exactly as approved and drawn in the workspace's
 * design/desktop-first-run/. Typographic apostrophes (spec A §6.2 rule). The session-ended words are spec A's
 * (`AuthExpiredBanner.tsx`), pinned equal by tests/accountViewCopy.test.ts. The menu-bar tooltips are Rust's
 * (`src-tauri/src/tray_presentation.rs`): no window runs while the app sits in the menu bar.
 */
import type { AccountNotice, NoticeLink } from './accountView'

export const SIGN_IN_TITLE = 'Sign in to Beebeeb'
export const SIGN_IN_WITH_BROWSER = 'Sign in with browser'
export const USE_EMAIL_AND_PASSWORD = 'Use email and password'
export const CREATE_ACCOUNT_PROMPT = 'New to Beebeeb?'
export const CREATE_ACCOUNT_LINK = 'Create an account on beebeeb.io'
export const QUIT_BEEBEEB = 'Quit Beebeeb'
export const OFFLINE_LINE = 'Can’t reach Beebeeb. Check your connection.'
export const TRY_AGAIN = 'Try again'
export const SESSION_ENDED_LINE = 'Sync is paused until you sign in again.'
export const SIGN_IN_AGAIN = 'Sign in again'
export const UPDATE_TITLE = 'Update Beebeeb'
export const UPDATE_BODY = 'This version of Beebeeb is too old to connect. Update to continue.'
export const UPDATE_NOW = 'Update now'
export const CHOOSE_PLAN_TITLE = 'Choose a plan'
export const CHOOSE_PLAN_BODY = 'Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.'
export const CHOOSE_PLAN_BUTTON = 'Choose a plan on beebeeb.io'
export const CHOOSE_PLAN_WAITING = 'Waiting for your plan. This window updates by itself.'
export const SIGN_OUT = 'Sign out'
export const COPY_LINK = 'Copy link'

export const NOTICE_LINK_LABEL: Readonly<Record<NoticeLink, string>> = {
  manage_plan: 'Manage plan',
  choose_plan: 'Choose a plan',
  contact_support: 'Contact support',
  update_payment_details: 'Update payment details',
}

/** §7.5: the server's words when it sent them, verbatim; otherwise the approved line, every clause whose date is
 *  missing dropped (never an invented date). `null`: nothing honest to say. */
export function noticeLine(notice: AccountNotice): string | null {
  if (notice.server_line) return notice.server_line
  switch (notice.kind) {
    case 'trial':
      return notice.trial_ends ? `Free trial until ${notice.trial_ends}.` : null
    case 'read_only': {
      const since = notice.read_only_since ? `Read-only since ${notice.read_only_since}.` : 'Read-only.'
      const deletion = notice.data_deletion_at ? `Files are deleted on ${notice.data_deletion_at} unless you choose a plan.` : 'Choose a plan to upload again.'
      return `${since} ${deletion}`
    }
    case 'read_only_other':
      return 'Read-only.'
    case 'frozen':
      return 'This account is frozen.'
    case 'payment_failed':
      return 'Payment failed. Update your payment details on beebeeb.io.'
  }
}
```

- [ ] **Step 4: `src/accountLinks.ts`**

```ts
/**
 * Spec 2026-10-07 §5.1: the built-in addresses of the account's web destinations. This is the only file in src/ that
 * contains them (tests/accountLinks.test.ts scans the rest). Rust resolves every link and puts it in the AccountView;
 * the built-in ones are used here only before a view has loaded.
 */
import type { AccountView } from './accountView'
import { openUrl, type CommandResult } from './desktopApi'

export const BUILT_IN_LINKS = Object.freeze({
  create_account: 'https://app.beebeeb.io/signup',
  billing: 'https://app.beebeeb.io/billing',
  support: 'https://beebeeb.io/support',
})

export type AccountDestination = 'create_account' | 'billing' | 'support' | 'step'

export function linkFor(view: AccountView | null | undefined, destination: AccountDestination): string {
  if (view) return view.links[destination]
  return destination === 'step' ? BUILT_IN_LINKS.billing : BUILT_IN_LINKS[destination]
}

export type LinkOpen = { opened: true } | { opened: false; url: string }

/** Opens exactly `url` in the system browser; when that fails, the caller shows the address with "Copy link" (§10). */
export async function openAccountLink(url: string, open: (url: string) => Promise<CommandResult<void>> = openUrl): Promise<LinkOpen> {
  const result = await open(url)
  return result.ok ? { opened: true } : { opened: false, url }
}

export async function copyAccountLink(url: string, write: (text: string) => Promise<void> = (text) => navigator.clipboard.writeText(text)): Promise<boolean> {
  try {
    await write(url)
    return true
  } catch {
    return false
  }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `bun test tests/accountView.test.ts tests/accountViewCopy.test.ts tests/accountLinks.test.ts > $EVID/t15-green.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t15-green.log`
Expected: `rc=0`, `17 pass`, `0 fail`.

- [ ] **Step 6: Mutation-check**

1. In `noticeLine`, replace the deletion fallback with `'Files are deleted soon unless you choose a plan.'`. Expected: `every read-only variant drops the clause…`. Revert.
2. In `parseAccountView`, drop the `oneOf(ACCOUNT_STATES, state)` check. Expected: `refuses anything that does not match the contract` (the `future_state` row). Revert.
3. In `linkFor`, return `BUILT_IN_LINKS.billing` for `step` even with a view. Expected: `linkFor reads the view…`. Revert.

- [ ] **Step 7: Commit**

```bash
cd $WT && bunx tsc --noEmit > $EVID/t15-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t15-eslint.log 2>&1; echo "rc=$?"
git add src/accountView.ts src/accountViewCopy.ts src/accountLinks.ts tests/accountView.test.ts tests/accountViewCopy.test.ts tests/accountLinks.test.ts
git commit -m "desktop: the account view's frontend contract, copy and links" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/accountView.ts src/accountViewCopy.ts src/accountLinks.ts tests/accountView.test.ts tests/accountViewCopy.test.ts tests/accountLinks.test.ts
git show --stat HEAD
```

---

## Task 16: The shared sign-in — browser first, password second, "Create account", offline (spec §7.1, C-S1)

**Lane T.** The 1734 device-code screen moves out of `WindowsFirstRun.tsx` into one component both windows mount. The browser result is read with the password path's parser. Carries Lane T's two rebase items when spec A left them open (Task 0): T11-M5 (step 8) and T11f1-c2 (the "couldn't confirm" sentence renders on both paths).

**Files:**
- Create: `src/browserSignIn.ts`, `src/BrowserSignIn.tsx`, `src/accountScreens.tsx`
- Modify: `src/browserLoginCopy.ts` (the panel's inline words move here, unchanged), `src/onboardingSignIn.ts` (`fresh.vaultUnlocked`; M5), `src/accountSwitchCopy.ts` (M5), `src/Onboarding.tsx` (`SignInStep` gains `onBack`; new `SignInMethods`, `LaunchLocationNotice`; the step flow's `afterSignIn`), `src/WindowsFirstRun.tsx` (mounts `BrowserSignIn`, the footer and the offline line), `src/design.css` (classes)
- Test: `tests/browserSignIn.test.ts`, `tests/signInMethods.test.tsx`; modify `tests/onboardingSignIn.test.ts`, `tests/reauthInPlace.test.tsx`, `tests/onboardingFinderStep.test.tsx` (`{ kind: 'fresh' }` gains `vaultUnlocked: false`; two Linux step-flow tests)

**Interfaces:**
- Consumes: Task 15's `AccountView`, `linkFor`, `openAccountLink`, `copyAccountLink`, `retryAccountView`, the copy; Task 12's `start_browser_login` result; spec A's `settledFrom`, `SignInSettled`, `SIGN_IN_OUTCOME_UNREADABLE`, `useFinderSetup`, `browserLoginCopy.ts`; command `quit_app` (Task 11).
- Produces:
  - `src/onboardingSignIn.ts`: `SignInSettled` = `{ kind: 'fresh'; vaultUnlocked: boolean } | { kind: 'reauthenticated'; vaultUnlocked: boolean } | { kind: 'account_mismatch'; pendingChanges: number }` (`number | null` after step 8)
  - `src/browserSignIn.ts`: `BROWSER_LOGIN_EVENT`, `BrowserPhase`, `BrowserLoginEvent`, `BrowserSignInState`, `BROWSER_IDLE`, `onBrowserEvent(state, event)`, `BrowserSignInResult`, `browserResult(result)`
  - `src/BrowserSignIn.tsx`: default export `BrowserSignIn({ onSettled })`
  - `src/accountScreens.tsx`: `AccountCard({ title, copy, children })`, `SignInFooter({ createAccountUrl, onOpen?, onQuit? })`, `OfflineLine({ busy, onRetry })`, `LinkFallback({ url, onCopy? })`, `useLinkOpener(open?: (url) => Promise<LinkOpen>): { openLink(url): Promise<void>; failedUrl: string | null; fallback: ReactNode }` (§10 "Browser won't open" at every link site, plan review I7: Tasks 17 and 18 use it everywhere a link opens)
  - `src/Onboarding.tsx`: `SignInMethods({ view, again, onSettled })`, `LaunchLocationNotice()`

- [ ] **Step 1: Move the panel's words into `browserLoginCopy.ts`**

Append (the words are today's, from `WindowsFirstRun.tsx`'s browser panel; the apostrophe becomes typographic):

```ts
/** The device-code panel's labels (task 1734's screen, shared by every sign-in window since spec 2026-10-07). */
export const BROWSER_OPENED_LABEL = 'We opened your browser'
export const BROWSER_AUTHORIZED_LABEL = 'Authorized — finishing…'
export const BROWSER_DID_NOT_OPEN_LEAD = 'Browser didn’t open? Visit'
export const BROWSER_BUSY_BUTTON = { connecting: 'Connecting…', waiting: 'Waiting for browser…', authorized: 'Finishing…' } as const
```

- [ ] **Step 2: Write the failing tests**

`tests/browserSignIn.test.ts`:

```ts
/**
 * C-S1 (spec 2026-10-07 §7.1): the browser sign-in returns the password path's `LoginOutcome` and is read by the same
 * `settledFrom`. A fresh browser sign-in carries the keys; another account reaches the switch warning; no raw code
 * reaches the screen; Rust's own error sentence (the "couldn't confirm" one, ruling T11f1-c2) is shown as it is.
 */
import { describe, expect, test } from 'bun:test'
import { BROWSER_IDLE, browserResult, onBrowserEvent } from '../src/browserSignIn'
import { settledFrom } from '../src/onboardingSignIn'
import { SIGN_IN_OUTCOME_UNREADABLE } from '../src/accountSwitchCopy'

const ACCOUNT_UNKNOWN = 'Beebeeb couldn’t confirm which account just signed in, so nothing changed on this computer. Connect to the internet and try again.'

describe('browserResult', () => {
  test('a fresh browser sign-in carries the keys', () => {
    const value = JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":true,"account_mismatch":null}')
    expect(browserResult({ ok: true, value })).toEqual({ ok: true, settled: { kind: 'fresh', vaultUnlocked: true } })
  })

  test('another account reaches the switch exactly as the password path does', () => {
    const value = JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":2}}')
    const settled = { kind: 'account_mismatch', pendingChanges: 2 }
    expect(browserResult({ ok: true, value })).toEqual({ ok: true, settled })
    expect(settledFrom(value)).toEqual(settled)
  })

  test('an error is the sentence Rust wrote, the account-unknown one included', () => {
    expect(browserResult({ ok: false, reason: ACCOUNT_UNKNOWN, unsupported: false })).toEqual({ ok: false, message: ACCOUNT_UNKNOWN })
  })

  test('an unreadable result is never a sign-in', () => {
    expect(browserResult({ ok: true, value: { requires_2fa: false, reauthenticated: 'yes' } as never })).toEqual({ ok: false, message: SIGN_IN_OUTCOME_UNREADABLE })
  })
})

describe('onBrowserEvent', () => {
  test('progress moves the phase; done means finishing; nothing here decides the outcome', () => {
    let state = onBrowserEvent(BROWSER_IDLE, { phase: 'connecting' })
    expect(state.phase).toBe('connecting')
    state = onBrowserEvent(state, { phase: 'waiting', user_code: 'ABCD-EFGH', verification_uri: 'https://example.invalid/verify' })
    expect(state).toEqual({ phase: 'waiting', userCode: 'ABCD-EFGH', verificationUri: 'https://example.invalid/verify', error: null })
    expect(onBrowserEvent(state, { phase: 'done' }).phase).toBe('authorized')
  })

  test('an error event never puts raw text on the screen', () => {
    const state = onBrowserEvent({ ...BROWSER_IDLE, phase: 'waiting' }, { phase: 'error', message: 'account_mismatch' })
    expect(state.error).toBeNull()
    expect(state.phase).toBe('waiting')
  })
})
```

`tests/signInMethods.test.tsx`:

```tsx
/**
 * Spec 2026-10-07 §7.1 and §7.2: the sign-in screen on macOS and Linux. Browser first, password second, "Create account"
 * opening exactly the view's address, the offline line in place of the buttons, and spec A's words in sign-in-again mode.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as copy from '../src/accountViewCopy'
import { BUILT_IN_LINKS, linkFor } from '../src/accountLinks'
import { BROWSER_SIGN_IN_INTRO } from '../src/browserLoginCopy'
import type { AccountView } from '../src/accountView'
import { loadComponent, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const LINKS = { create_account: 'https://app.beebeeb.io/signup?from=server', billing: 'https://app.beebeeb.io/billing', support: 'https://beebeeb.io/support', step: 'https://app.beebeeb.io/billing' }
const signedOut = (fields: Partial<AccountView> = {}): AccountView =>
  ({ state: 'signed_out', session: 'none', offline: false, screen: 'sign_in', busy: false, links: LINKS, notice: null, ...fields }) as AccountView

const AccountCard = loadComponent('accountScreens.tsx', 'AccountCard', { React, Wordmark: () => null })

function methods(view: AccountView | null, again = false) {
  const stubs: Record<string, (props: any) => null> = {}
  for (const name of ['SignInStep', 'SignInFooter', 'LaunchLocationNotice', 'OfflineLine', 'BrowserSignIn']) stubs[name] = () => null
  const settled: unknown[] = []
  const m = mount('Onboarding.tsx', 'SignInMethods', {
    backend: { account_view_retry: () => null },
    props: { view, again, onSettled: (s: unknown) => settled.push(s) },
    bindings: {
      ...stubs, AccountCard, linkFor, retryAccountView: async () => ({ ok: true, value: undefined }),
      SIGN_IN_TITLE: copy.SIGN_IN_TITLE, SIGN_IN_AGAIN: copy.SIGN_IN_AGAIN, SESSION_ENDED_LINE: copy.SESSION_ENDED_LINE,
      USE_EMAIL_AND_PASSWORD: copy.USE_EMAIL_AND_PASSWORD, BROWSER_SIGN_IN_INTRO,
    },
  })
  mounted.push(m)
  const find = (name: string) => m.elements().find((el) => el.type === stubs[name])
  return { m, find, settled }
}

describe('SignInMethods', () => {
  test('browser first, the password as a text link, and the footer with the view’s address', () => {
    const v = methods(signedOut())
    const card = v.m.elements().find((el) => el.type === AccountCard)!
    expect(card.props.title).toBe(copy.SIGN_IN_TITLE)
    expect(v.find('BrowserSignIn')).toBeDefined()
    expect(v.m.elements().some((el) => el.type === 'button' && textOf(el.props.children) === copy.USE_EMAIL_AND_PASSWORD)).toBe(true)
    expect(v.find('SignInFooter')!.props.createAccountUrl).toBe(LINKS.create_account)
    expect(v.find('SignInStep')).toBeUndefined()
  })

  test('the text link opens today’s email and password flow, which can go back', async () => {
    const v = methods(signedOut())
    await v.m.click(copy.USE_EMAIL_AND_PASSWORD)
    const step = v.find('SignInStep')!
    expect(step).toBeDefined()
    expect(v.find('BrowserSignIn')).toBeUndefined()
    expect(v.find('SignInFooter')).toBeDefined()
    step.props.onDone({ kind: 'fresh', vaultUnlocked: false })
    expect(v.settled).toEqual([{ kind: 'fresh', vaultUnlocked: false }])
    step.props.onBack(); v.m.render()
    expect(v.find('BrowserSignIn')).toBeDefined()
  })

  test('offline: the offline line replaces both methods; Create account stays', () => {
    const v = methods(signedOut({ offline: true }))
    expect(v.find('OfflineLine')).toBeDefined()
    expect(v.find('BrowserSignIn')).toBeUndefined()
    expect(v.m.elements().some((el) => el.type === 'button' && textOf(el.props.children) === copy.USE_EMAIL_AND_PASSWORD)).toBe(false)
    expect(v.find('SignInFooter')).toBeDefined()
  })

  test('sign-in-again mode uses spec A’s words', () => {
    const v = methods(signedOut({ state: 'session_ended', session: 'ended', screen: 'sign_in_again' }), true)
    const card = v.m.elements().find((el) => el.type === AccountCard)!
    expect([card.props.title, card.props.copy]).toEqual([copy.SIGN_IN_AGAIN, copy.SESSION_ENDED_LINE])
  })

  test('before the view has loaded, Create account is the built-in address', () => {
    expect(methods(null).find('SignInFooter')!.props.createAccountUrl).toBe(BUILT_IN_LINKS.create_account)
  })
})

describe('SignInFooter', () => {
  function footer(open: (url: string) => Promise<{ opened: true } | { opened: false; url: string }>) {
    const quits: number[] = []
    const LinkFallback = () => null
    const m = mount('accountScreens.tsx', 'SignInFooter', {
      backend: {},
      props: { createAccountUrl: LINKS.create_account, onOpen: open, onQuit: async () => { quits.push(1) } },
      // The real `useLinkOpener` runs inside this mount (plan review I7: the one fallback every link site uses).
      hookModules: [{ file: 'accountScreens.tsx', name: 'useLinkOpener', bindings: { LinkFallback, openAccountLink: open } }],
      bindings: { CREATE_ACCOUNT_PROMPT: copy.CREATE_ACCOUNT_PROMPT, CREATE_ACCOUNT_LINK: copy.CREATE_ACCOUNT_LINK, QUIT_BEEBEEB: copy.QUIT_BEEBEEB },
    })
    mounted.push(m)
    const link = () => m.elements().find((el) => el.type === 'a')!
    return { m, link, quits, LinkFallback }
  }

  test('Create account opens exactly the given address', async () => {
    const opened: string[] = []
    const f = footer(async (url) => { opened.push(url); return { opened: true } })
    expect(textOf(f.link().props.children)).toBe(copy.CREATE_ACCOUNT_LINK)
    expect(f.link().props.href).toBe(LINKS.create_account)
    await f.link().props.onClick({ preventDefault() {} }); await f.m.flush()
    expect(opened).toEqual([LINKS.create_account])
    expect(f.m.elements().some((el) => el.type === f.LinkFallback)).toBe(false)
  })

  test('when the browser does not open, the address and Copy link show', async () => {
    const f = footer(async (url) => ({ opened: false, url }))
    await f.link().props.onClick({ preventDefault() {} }); await f.m.flush()
    expect(f.m.elements().find((el) => el.type === f.LinkFallback)!.props.url).toBe(LINKS.create_account)
  })

  test('Quit Beebeeb quits', async () => {
    const f = footer(async () => ({ opened: true }))
    await f.m.click(copy.QUIT_BEEBEEB)
    expect(f.quits).toEqual([1])
  })
})

describe('the shared screen is mounted by every sign-in window', () => {
  test('Windows mounts BrowserSignIn and no longer runs its own handoff', () => {
    const windows = readFileSync(new URL('../src/WindowsFirstRun.tsx', import.meta.url), 'utf8')
    expect(windows).toContain("import BrowserSignIn from './BrowserSignIn'")
    expect(windows).not.toContain("command<void>('start_browser_login')")
    expect(windows).toContain('<SignInFooter')
    const onboarding = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
    expect(onboarding).toContain('<BrowserSignIn')
  })
})
```

Update the existing tests for the new `fresh` shape and the new sign-in step: in `tests/onboardingSignIn.test.ts`, `tests/reauthInPlace.test.tsx` and `tests/onboardingFinderStep.test.tsx`, every `{ kind: 'fresh' }` becomes `{ kind: 'fresh', vaultUnlocked: false }`. The step flow now mounts `SignInMethods` for its sign-in step, so in `tests/reauthInPlace.test.tsx` and `tests/onboardingFinderStep.test.tsx` add `'SignInMethods'` to the stubbed step names and replace each `find('SignInStep')!.props.onDone(x)` (or `props('SignInStep').onDone(x)`) with `find('SignInMethods')!.props.onSettled(x)` (`props('SignInMethods').onSettled(x)`); the assertions stay. In `tests/onboardingFinderStep.test.tsx`, add `SignInMethods: make('SignInMethods'),` to `stubs()` and these two tests (plan review I6: the Linux step flow is the one consumer of `fresh.vaultUnlocked`; spec §14 "a fresh browser result with `vault_unlocked: true` goes on, never to `UnlockStep`"). If `OnboardingView` on Linux asks a command these do not script, script it the way the file's existing Linux tests do.

```tsx
describe('the Linux step flow after a sign-in (C-S1)', () => {
  const signedOut = () => ({ logged_in: false, engine: 'idle', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0 })

  test('a fresh browser result that brought the keys skips the recovery phrase', async () => {
    const o = await openOnboarding({ sync_status: () => signedOut(), desktop_platform: () => 'linux' })
    expect(o.shown()).toEqual(['SignInMethods'])
    o.props('SignInMethods').onSettled({ kind: 'fresh', vaultUnlocked: true }); o.m.render()
    expect(o.shown()).toEqual(['FinderInstallStep'])
  })

  test('a fresh result without the keys asks for the recovery phrase', async () => {
    const o = await openOnboarding({ sync_status: () => signedOut(), desktop_platform: () => 'linux' })
    o.props('SignInMethods').onSettled({ kind: 'fresh', vaultUnlocked: false }); o.m.render()
    expect(o.shown()).toEqual(['UnlockStep'])
  })
})
```

Then add to `tests/onboardingSignIn.test.ts`'s `settledFrom (R8)` block:

```ts
  test('a fresh result keeps vault_unlocked (C-S1): the browser path delivers the keys', () => {
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":true,"account_mismatch":null}')))
      .toEqual({ kind: 'fresh', vaultUnlocked: true })
    expect(settledFrom({ requires_2fa: false })).toEqual({ kind: 'fresh', vaultUnlocked: false })
  })
```

Run: `bun test tests/browserSignIn.test.ts tests/signInMethods.test.tsx tests/onboardingSignIn.test.ts tests/onboardingFinderStep.test.tsx > $EVID/t16-red.log 2>&1; echo "rc=$?"`. Expected: `rc=1` with module-not-found, `settledFrom` mismatches and the Linux flow showing `SignInStep`; paste them.

- [ ] **Step 3: `settledFrom` keeps the keys of a fresh result**

In `src/onboardingSignIn.ts`:

```ts
/** What a completed sign-in became (R8). `vaultUnlocked` on a fresh sign-in: the browser handoff delivers the keys
 *  with the session (spec 2026-10-07 C-S1), so no recovery phrase is needed. */
export type SignInSettled =
  | { kind: 'fresh'; vaultUnlocked: boolean }
  | { kind: 'reauthenticated'; vaultUnlocked: boolean }
  | { kind: 'account_mismatch'; pendingChanges: number }
```

and in `settledFrom`: `if (value === null || value === undefined) return { kind: 'fresh', vaultUnlocked: false }`, and the last line `return { kind: 'fresh', vaultUnlocked: vault_unlocked === true }`. Update the doc comment's bullet list with "a fresh result keeps `vault_unlocked` (C-S1)".

- [ ] **Step 4: `src/browserSignIn.ts` and `src/BrowserSignIn.tsx`**

```ts
/**
 * The browser handoff (task 1734), shared by every sign-in window since spec 2026-10-07 (§7.1, C-S1). The command
 * `start_browser_login` returns the same `LoginOutcome` as the password path and is read by the same strict
 * `settledFrom`, so another account reaches the switch warning the same way and no raw code reaches the screen.
 * Progress arrives as `browser-login` events; only the command's own result decides the outcome.
 */
import { settledFrom, type SignInSettled } from './onboardingSignIn'
import { SIGN_IN_OUTCOME_UNREADABLE } from './accountSwitchCopy'
import { commandUnavailableLabel, type CommandResult, type DesktopLoginResult } from './desktopApi'

export const BROWSER_LOGIN_EVENT = 'browser-login'

export type BrowserPhase = 'idle' | 'connecting' | 'waiting' | 'authorized' | 'error'

export interface BrowserLoginEvent {
  phase: 'connecting' | 'waiting' | 'authorized' | 'done' | 'error'
  user_code?: string
  verification_uri?: string
  message?: string
}

export interface BrowserSignInState {
  phase: BrowserPhase
  userCode: string | null
  verificationUri: string | null
  error: string | null
}

export const BROWSER_IDLE: BrowserSignInState = { phase: 'idle', userCode: null, verificationUri: null, error: null }

/** Progress only. `done` means finishing; an `error` event changes nothing (the command's `Err` is the one error). */
export function onBrowserEvent(state: BrowserSignInState, event: BrowserLoginEvent): BrowserSignInState {
  switch (event.phase) {
    case 'connecting':
      return { ...state, phase: 'connecting', error: null }
    case 'waiting':
      return { ...state, phase: 'waiting', userCode: event.user_code ?? state.userCode, verificationUri: event.verification_uri ?? state.verificationUri }
    case 'authorized':
    case 'done':
      return { ...state, phase: 'authorized' }
    case 'error':
      return state
  }
}

export type BrowserSignInResult = { ok: true; settled: SignInSettled } | { ok: false; message: string }

export function browserResult(result: CommandResult<DesktopLoginResult | null>): BrowserSignInResult {
  if (!result.ok) return { ok: false, message: result.unsupported ? commandUnavailableLabel('start_browser_login') : result.reason }
  const settled = settledFrom(result.value)
  return settled.kind === 'unreadable' ? { ok: false, message: SIGN_IN_OUTCOME_UNREADABLE } : { ok: true, settled }
}
```

```tsx
/**
 * The 1734 device-code screen (words unchanged, in browserLoginCopy.ts), mounted by the macOS and Linux sign-in
 * (Onboarding.tsx) and by Windows (WindowsFirstRun.tsx). The code is machine text, in mono.
 */
import { useCallback, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { command, type DesktopLoginResult } from './desktopApi'
import {
  BROWSER_AUTHORIZED_LABEL,
  BROWSER_BUSY_BUTTON,
  BROWSER_DID_NOT_OPEN_HINT,
  BROWSER_DID_NOT_OPEN_LEAD,
  BROWSER_OPENED_LABEL,
  browserWaitingInstruction,
} from './browserLoginCopy'
import { SIGN_IN_WITH_BROWSER, TRY_AGAIN } from './accountViewCopy'
import { BROWSER_IDLE, BROWSER_LOGIN_EVENT, browserResult, onBrowserEvent, type BrowserLoginEvent, type BrowserSignInState } from './browserSignIn'
import type { SignInSettled } from './onboardingSignIn'

export default function BrowserSignIn({ onSettled }: { onSettled: (settled: SignInSettled) => void }) {
  const [state, setState] = useState<BrowserSignInState>(BROWSER_IDLE)
  const running = useRef(false)
  const busy = state.phase === 'connecting' || state.phase === 'waiting' || state.phase === 'authorized'

  const start = useCallback(async () => {
    if (running.current) return
    running.current = true
    setState({ ...BROWSER_IDLE, phase: 'connecting' })
    const unlisten = await listen<BrowserLoginEvent>(BROWSER_LOGIN_EVENT, (event) => setState((current) => onBrowserEvent(current, event.payload)))
    const result = browserResult(await command<DesktopLoginResult | null>('start_browser_login'))
    unlisten()
    running.current = false
    if (!result.ok) {
      setState((current) => ({ ...current, phase: 'error', error: result.message }))
      return
    }
    setState(BROWSER_IDLE)
    onSettled(result.settled)
  }, [onSettled])

  const label = state.phase === 'error' ? TRY_AGAIN : busy ? BROWSER_BUSY_BUTTON[state.phase as 'connecting' | 'waiting' | 'authorized'] : SIGN_IN_WITH_BROWSER
  return (
    <div className="browser-sign-in">
      {state.error ? <div className="notice error" role="alert" data-error-surface="sign-in">{state.error}</div> : null}
      {state.phase === 'waiting' || state.phase === 'authorized' ? (
        <div className="device-code-panel">
          <div className="device-code-label">{state.phase === 'authorized' ? BROWSER_AUTHORIZED_LABEL : BROWSER_OPENED_LABEL}</div>
          <p className="device-code-instruction">{browserWaitingInstruction(state.phase)}</p>
          {state.userCode ? <div className="device-code mono">{state.userCode}</div> : null}
          {state.verificationUri && state.phase === 'waiting' ? (
            <p className="device-code-hint">
              {BROWSER_DID_NOT_OPEN_LEAD} <span className="mono">{state.verificationUri}</span> {BROWSER_DID_NOT_OPEN_HINT}
            </p>
          ) : null}
        </div>
      ) : null}
      <button className="button amber" disabled={busy} onClick={() => void start()}>
        {label}
      </button>
    </div>
  )
}
```

- [ ] **Step 5: `src/accountScreens.tsx` (the parts the sign-in needs)**

```tsx
/**
 * Spec 2026-10-07 §7: the account window's presentational pieces, shared by macOS, Linux and Windows. Words from
 * accountViewCopy.ts; links opened exactly as the account view names them (accountLinks.ts).
 */
import { useState, type ReactNode } from 'react'
import { command, type CommandResult } from './desktopApi'
import { Wordmark } from './Logo'
import { COPY_LINK, CREATE_ACCOUNT_LINK, CREATE_ACCOUNT_PROMPT, OFFLINE_LINE, QUIT_BEEBEEB, TRY_AGAIN } from './accountViewCopy'
import { copyAccountLink, openAccountLink, type LinkOpen } from './accountLinks'

export function AccountCard({ title, copy, children }: { title: string; copy?: string; children?: ReactNode }) {
  return (
    <section className="auth-card account-card">
      <div className="auth-card-header">
        <Wordmark className="auth-logo" />
      </div>
      <h1 className="page-title">{title}</h1>
      {copy ? <p className="page-copy">{copy}</p> : null}
      {children}
    </section>
  )
}

/** §10: the address as selectable text, and "Copy link". */
export function LinkFallback({ url, onCopy = copyAccountLink }: { url: string; onCopy?: (url: string) => Promise<boolean> }) {
  return (
    <div className="link-fallback">
      <code className="mono selectable">{url}</code>
      <button className="button" onClick={() => void onCopy(url)}>
        {COPY_LINK}
      </button>
    </div>
  )
}

/** §7.1: in place of the buttons while offline (network level only). */
export function OfflineLine({ busy, onRetry }: { busy: boolean; onRetry: () => void }) {
  return (
    <div className="offline-line" role="status">
      <p className="page-copy">{OFFLINE_LINE}</p>
      <button className="button" disabled={busy} onClick={onRetry}>
        {TRY_AGAIN}
      </button>
    </div>
  )
}

/**
 * §10 "Browser won't open", the one way every link site opens an address (plan review I7): the system browser, and when
 * that fails, `fallback` is the address as selectable text with "Copy link", rendered under the link. A later open that
 * works clears it.
 */
export function useLinkOpener(open: (url: string) => Promise<LinkOpen> = openAccountLink): {
  openLink: (url: string) => Promise<void>
  failedUrl: string | null
  fallback: ReactNode
} {
  const [failed, setFailed] = useState<string | null>(null)
  const openLink = async (url: string) => {
    const result = await open(url)
    setFailed(result.opened ? null : result.url)
  }
  return { openLink, failedUrl: failed, fallback: failed ? <LinkFallback url={failed} /> : null }
}

/** §7.1: "New to Beebeeb? Create an account on beebeeb.io" (the account view's address, system browser) and "Quit Beebeeb". */
export function SignInFooter({
  createAccountUrl,
  onOpen = openAccountLink,
  onQuit = () => command<void>('quit_app'),
}: {
  createAccountUrl: string
  onOpen?: (url: string) => Promise<LinkOpen>
  onQuit?: () => Promise<CommandResult<void> | void>
}) {
  const { openLink, fallback } = useLinkOpener(onOpen)
  return (
    <footer className="sign-in-footer">
      <p className="page-copy">
        {CREATE_ACCOUNT_PROMPT}{' '}
        <a className="text-link" href={createAccountUrl} onClick={(event) => { event.preventDefault(); void openLink(createAccountUrl) }}>
          {CREATE_ACCOUNT_LINK}
        </a>
      </p>
      {fallback}
      <button className="text-link quiet" onClick={() => void onQuit()}>
        {QUIT_BEEBEEB}
      </button>
    </footer>
  )
}
```

- [ ] **Step 6: `SignInMethods` and the step flow in `Onboarding.tsx`**

`SignInStep` takes `{ onDone, onBack }: { onDone: (settled: SignInSettled) => void; onBack?: () => void }` and, in its password mode, renders after the `</form>`:

```tsx
      {onBack ? (
        <div className="button-row" style={{ marginTop: 12 }}>
          <button className="button" onClick={onBack} disabled={busy}>
            ← Back
          </button>
        </div>
      ) : null}
```

(its other copy, "Welcome back" and "Sign in to unlock your encrypted vault.", is unchanged).

New, after `SignInStep`:

```tsx
/**
 * Spec 2026-10-07 §7.1 (macOS and Linux): browser first, password second, "Create account" on the web, and, while
 * offline, the offline line in place of both. `again`: spec A's sign-in-again words (§7.2).
 */
function SignInMethods({ view, again, onSettled }: { view: AccountView | null; again: boolean; onSettled: (settled: SignInSettled) => void }) {
  const [method, setMethod] = useState<'browser' | 'password'>('browser')
  const [retrying, setRetrying] = useState(false)
  const footer = <SignInFooter createAccountUrl={linkFor(view, 'create_account')} />
  if (method === 'password') {
    return (
      <>
        <SignInStep onDone={onSettled} onBack={() => setMethod('browser')} />
        {footer}
      </>
    )
  }
  const retry = async () => {
    setRetrying(true)
    await retryAccountView()
    setRetrying(false)
  }
  return (
    <AccountCard title={again ? SIGN_IN_AGAIN : SIGN_IN_TITLE} copy={again ? SESSION_ENDED_LINE : BROWSER_SIGN_IN_INTRO}>
      <LaunchLocationNotice />
      {view?.offline ? (
        <OfflineLine busy={retrying} onRetry={() => void retry()} />
      ) : (
        <>
          <BrowserSignIn onSettled={onSettled} />
          <button className="text-link" onClick={() => setMethod('password')}>
            {USE_EMAIL_AND_PASSWORD}
          </button>
        </>
      )}
      {footer}
    </AccountCard>
  )
}

/** §7.1: spec A's `not_in_applications` sentence and "Show in Finder", above the buttons, before anyone signs in
 *  (macOS; elsewhere `finder_setup_state` answers nothing and this renders nothing). */
function LaunchLocationNotice() {
  const finder = useFinderSetup()
  const presentation = finder.presentation
  if (presentation.kind !== 'notice' || presentation.action !== 'show_in_finder') return null
  return (
    <div className="notice" role="status" style={{ marginBottom: 14 }}>
      <div>{presentation.sentence}</div>
      <div className="button-row" style={{ marginTop: 10 }}>
        <button className="button" onClick={() => void finder.run('show_in_finder')}>
          {presentation.actionLabel}
        </button>
      </div>
    </div>
  )
}
```

Imports to add to `Onboarding.tsx`: `AccountView` and `retryAccountView` from `./accountView`; `linkFor` from `./accountLinks`; `SIGN_IN_TITLE`, `SIGN_IN_AGAIN`, `SESSION_ENDED_LINE`, `USE_EMAIL_AND_PASSWORD` from `./accountViewCopy`; `BROWSER_SIGN_IN_INTRO` from `./browserLoginCopy`; `AccountCard`, `OfflineLine`, `SignInFooter` from `./accountScreens`; `BrowserSignIn` from `./BrowserSignIn`.

In the step flow (`OnboardingView`, used by Linux and an unknown platform; macOS moves to Task 17's window), mount `SignInMethods` for the sign-in step and let a fresh browser sign-in skip the recovery phrase:

```tsx
        {step === 'signin' && <SignInMethods view={null} again={mode === 'reauth'} onSettled={afterSignIn} />}
```

```ts
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
    // C-S1: a fresh browser sign-in delivered the keys, so there is no recovery phrase to ask for.
    setStep(settled.kind === 'fresh' && settled.vaultUnlocked ? 'finder' : 'unlock')
  }
```

(`view={null}` here: the Linux step flow does not subscribe to the account view; its "Create account" uses the built-in address and it has no offline line until Task 17 gives `OnboardingView` the view for every platform. Task 17 replaces `null` with the view.)

- [ ] **Step 7: Windows mounts the shared screen**

In `WindowsFirstRun.tsx`'s `SignInStep` (browser mode): delete `startBrowserLogin`, the `browserPhase`/`userCode`/`verificationUri`/`browserDoneRef`/`browserError` state and the device-code panel JSX, and render in their place:

```tsx
      {view?.offline ? (
        <OfflineLine busy={retrying} onRetry={() => void retry()} />
      ) : (
        <>
          <BrowserSignIn onSettled={(settled) => {
            // Windows signs in only when no session exists, so another account cannot arrive here; if a result says so
            // anyway it is shown as unreadable, never as raw text.
            if (settled.kind === 'account_mismatch') { setPwError(SIGN_IN_OUTCOME_UNREADABLE); setMode('password'); return }
            onDone({ vaultUnlocked: settled.vaultUnlocked })
          }} />
          <Btn variant="ghost" onClick={() => setMode('password')}>{USE_EMAIL_AND_PASSWORD}</Btn>
        </>
      )}
      <SignInFooter createAccountUrl={linkFor(view, 'create_account')} />
```

with, at the top of the component, `const load = useAccountView(); const view = load.status === 'ready' ? load.view : null; const [retrying, setRetrying] = useState(false); const retry = async () => { setRetrying(true); await retryAccountView(); setRetrying(false) }`. The password and TOTP modes keep their forms and words, and gain the same `<SignInFooter … />` under them. The "Step 1 of 4" label, the heading "Sign in to Beebeeb" and `BROWSER_SIGN_IN_INTRO` stay. Imports: `BrowserSignIn`, `SignInFooter`, `OfflineLine`, `useAccountView`, `retryAccountView`, `linkFor`, `USE_EMAIL_AND_PASSWORD`, `SIGN_IN_OUTCOME_UNREADABLE`. Remove the imports this leaves unused (`listen`, `BROWSER_DID_NOT_OPEN_HINT`, `browserWaitingInstruction`, and the `BrowserLoginEvent` type) unless something else in the file still uses them; `bun run lint` names them.

- [ ] **Step 8 (only if Task 0 found T11-M5 open): an uncountable switch**

`src/onboardingSignIn.ts`: `| { kind: 'account_mismatch'; pendingChanges: number | null }`, and in `settledFrom` accept `pending === null`:

```ts
    if (pending === null) return { kind: 'account_mismatch', pendingChanges: null }
```

(before the `typeof pending !== 'number'` check). `src/accountSwitchCopy.ts`:

```ts
export function accountSwitchBody(pendingChanges: number | null): string {
  if (pendingChanges === null) {
    // T11-M5: the count could not be read. Never "0". Approved with the mocks (plan "Spec issues" 16).
    return 'This Mac is signed in to another Beebeeb account. Beebeeb couldn’t count the changes on this Mac that haven’t uploaded yet. Switching signs that account out of this Mac and removes any that are left.'
  }
  // … the three existing branches, unchanged
}
```

`src/desktopApi.ts`'s `DesktopLoginResult.account_mismatch` becomes `{ pending_changes: number | null } | null`. Tests, in `tests/onboardingSignIn.test.ts` and `tests/reauthInPlace.test.tsx`:

```ts
  test('an uncountable switch is read as unknown, never as zero (T11-M5)', () => {
    expect(settledFrom(JSON.parse('{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"account_mismatch":{"pending_changes":null}}')))
      .toEqual({ kind: 'account_mismatch', pendingChanges: null })
  })
```

```ts
  test('the switch warning never claims zero changes when it could not count them', () => {
    expect(switchCopy.accountSwitchBody(null)).not.toContain('0 ')
    expect(switchCopy.accountSwitchBody(null)).toContain('couldn’t count')
  })
```

If the lead changed the sentence when approving the mocks, use the approved words in both places.

- [ ] **Step 9: CSS**

Append to `src/design.css` (tokens only, no literal colours; `tests/noHardcodedThemeColors.test.ts` checks):

```css
/* Spec 2026-10-07: the shared sign-in and the account window's pieces. */
.text-link { background: none; border: 0; padding: 0; color: var(--ink-2); font: inherit; text-decoration: underline; cursor: pointer; }
.text-link.quiet { color: var(--ink-3); font-size: 12px; }
.sign-in-footer { margin-top: 22px; display: grid; gap: 8px; }
.browser-sign-in { display: grid; gap: 12px; margin-bottom: 12px; }
.device-code-panel { padding: 16px 18px; background: var(--amber-bg); border: 1px solid var(--amber-deep); border-radius: 10px; }
.device-code-label { font: 500 11px var(--font-mono); text-transform: uppercase; letter-spacing: 0.07em; color: var(--amber-deep); margin-bottom: 8px; }
.device-code { font-family: var(--font-mono); font-size: 26px; font-weight: 700; letter-spacing: 0.12em; color: var(--ink); margin-bottom: 10px; }
.device-code-hint { margin: 0; font-size: 11px; color: var(--ink-3); word-break: break-all; }
.offline-line { display: grid; gap: 10px; margin: 8px 0 12px; }
.link-fallback { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
.link-fallback .selectable { user-select: text; -webkit-user-select: text; }
```

If the token names differ in `design.css` (`--font-mono`, `--amber-bg`, `--amber-deep`, `--ink-2`, `--ink-3`), use the file's own names. The approved mocks decide spacing; adjust the numbers to them.

- [ ] **Step 10: Run, mutation-check, commit**

```bash
cd $WT && bun test > $EVID/t16-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t16-all.log
```

Expected: `rc=0`, `0 fail`, and the pass count is `B_bun` + 35 (Global Constraints: Task 15's 17 plus this task's 18, which are `browserSignIn` 6, `signInMethods` 9, `onboardingSignIn` 1, the Linux flow 2; + 2 more when step 8 ran). Mutations (paste each failure, revert):
1. `settledFrom` returns `{ kind: 'fresh', vaultUnlocked: false }` always. Expected: `a fresh browser sign-in carries the keys`, `a fresh result keeps vault_unlocked` and `a fresh browser result that brought the keys skips the recovery phrase`.
2. `onBrowserEvent`'s `'error'` case returns `{ ...state, error: event.message ?? null }`. Expected: `an error event never puts raw text on the screen`.
3. In `SignInMethods`, render `BrowserSignIn` even when offline. Expected: `offline: the offline line replaces both methods`.
4. In `afterSignIn`, `setStep('unlock')` for every fresh result. Expected: `a fresh browser result that brought the keys skips the recovery phrase`.
5. In `useLinkOpener`, `setFailed(null)` always. Expected: `when the browser does not open, the address and Copy link show`.

```bash
bunx tsc --noEmit > $EVID/t16-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t16-eslint.log 2>&1; echo "rc=$?"
git add src/browserSignIn.ts src/BrowserSignIn.tsx src/accountScreens.tsx src/browserLoginCopy.ts src/onboardingSignIn.ts src/accountSwitchCopy.ts src/desktopApi.ts src/Onboarding.tsx src/WindowsFirstRun.tsx src/design.css tests/browserSignIn.test.ts tests/signInMethods.test.tsx tests/onboardingSignIn.test.ts tests/reauthInPlace.test.tsx tests/onboardingFinderStep.test.tsx
git commit -m "desktop: one sign-in screen for every window: browser first, password second, create account on the web" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/browserSignIn.ts src/BrowserSignIn.tsx src/accountScreens.tsx src/browserLoginCopy.ts src/onboardingSignIn.ts src/accountSwitchCopy.ts src/desktopApi.ts src/Onboarding.tsx src/WindowsFirstRun.tsx src/design.css tests/browserSignIn.test.ts tests/signInMethods.test.tsx tests/onboardingSignIn.test.ts tests/reauthInPlace.test.tsx tests/onboardingFinderStep.test.tsx
git show --stat HEAD
```

(Drop `src/desktopApi.ts` from both lists if step 8 did not run.)

---

## Task 17: The macOS account window — every screen, the removals, after sign-in (spec §6.2 C-W8, §7, §11)

**Lane T.** On macOS the `onboarding` window becomes the account window: it renders the view's screen and nothing else. The Finder step, the pinning and ready steps and the rail go on macOS (§11); Linux keeps its step flow.

**Files:**
- Modify: `src/Onboarding.tsx` (new exported `MacAccountWindow`, `AfterSignInFinder`; delete `MacFinderStep`; `OnboardingView` loses its macOS branches and passes the view to `SignInMethods`)
- Modify: `src/accountScreens.tsx` (`UpdateScreen`, `ChoosePlanScreen`, `BusyScreen`, `AccountUpdate`, `AccountChoosePlan`, `accountWindowTitle`)
- Modify: `src/main.tsx` (`?window=onboarding&platform=macos` mounts `MacAccountWindow`)
- Modify: `src/design.css`
- Create: `src-tauri/capabilities/account-window.json` (plan review I2: `setTitle` and `close` for the `onboarding` window)
- Test: `tests/macAccountWindow.test.tsx`, `tests/accountScreens.test.tsx` (new); `tests/reauthInPlace.test.tsx`, `tests/onboardingFinderStep.test.tsx` (modified, see step 6)

**Interfaces:**
- Consumes: Task 15 (`useAccountView`, `AccountView`, `AccountScreen`, `retryAccountView`, `accountViewPlanOpened`, copy, `openAccountLink`); Task 16 (`SignInMethods`, `AccountCard`, `OfflineLine`, `LinkFallback`, `useLinkOpener`); Task 13 (the URL tag, `show_settings_on_account`); spec A (`UnlockStep`, `AccountSwitchStep`, `useFinderSetup`, `FINDER_SETUP_TITLE`, `desktopUpdateCheck`, the default export `ManualUpdateFeedback`, `ACCOUNT_SWITCH_FAILED`).
- Produces:
  - `src/Onboarding.tsx`: `export function MacAccountWindow()`
  - `src-tauri/capabilities/account-window.json`: `core:window:allow-set-title` and `core:window:allow-close` for the window labelled `onboarding` (`core:window:default` grants neither)
  - `src/accountScreens.tsx`: `UpdateScreen({ offline, busy, onUpdate, onRetry })`, `ChoosePlanScreen({ offline, waiting, busy, fallbackUrl, onChoose, onRetry, onSignOut })`, `BusyScreen()`, `AccountUpdate({ view })`, `AccountChoosePlan({ view, open?, planOpened?, retry?, signOut? })`, `accountWindowTitle(screen: AccountScreen): string` (Task 19 reuses the first five on Windows)

- [ ] **Step 1: Write the failing tests**

`tests/accountScreens.test.tsx`:

```tsx
/** Spec 2026-10-07 §7.3, §7.4, §10: the update and choose-a-plan screens, the busy screen, and the window title. */
import { afterEach, describe, expect, test } from 'bun:test'
import * as copy from '../src/accountViewCopy'
import type { AccountView } from '../src/accountView'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const STEP = 'https://app.beebeeb.io/billing?step=choose_plan'
const noPlan = (offline = false) =>
  ({ state: 'no_plan', session: 'valid', offline, screen: 'choose_plan', busy: false, notice: null,
     links: { create_account: 'https://app.beebeeb.io/signup', billing: 'https://app.beebeeb.io/billing', support: 'https://beebeeb.io/support', step: STEP } }) as AccountView

const stubs = () => ({ OfflineLine: () => null, LinkFallback: () => null, AccountCard: (props: any) => props.children, ManualUpdateFeedback: () => null })
const words = {
  UPDATE_TITLE: copy.UPDATE_TITLE, UPDATE_BODY: copy.UPDATE_BODY, UPDATE_NOW: copy.UPDATE_NOW, CHOOSE_PLAN_TITLE: copy.CHOOSE_PLAN_TITLE,
  CHOOSE_PLAN_BODY: copy.CHOOSE_PLAN_BODY, CHOOSE_PLAN_BUTTON: copy.CHOOSE_PLAN_BUTTON, CHOOSE_PLAN_WAITING: copy.CHOOSE_PLAN_WAITING, SIGN_OUT: copy.SIGN_OUT,
}

describe('AccountChoosePlan', () => {
  function choosePlan(view: AccountView, open: (url: string) => Promise<any>, signOut = async () => ({ ok: true, value: undefined })) {
    const order: string[] = []
    const s = stubs()
    const ChoosePlanScreen = (props: any) => ({ type: 'ChoosePlanScreen', props })
    const m = mount('accountScreens.tsx', 'AccountChoosePlan', {
      backend: {},
      props: {
        view,
        open: async (url: string) => { order.push(`open ${url}`); return open(url) },
        planOpened: async () => { order.push('plan_opened'); return { ok: true, value: undefined } },
        retry: async () => { order.push('retry'); return { ok: true, value: undefined } },
        signOut: async () => { order.push('sign_out'); return signOut() },
      },
      hookModules: [{ file: 'accountScreens.tsx', name: 'useLinkOpener', bindings: { LinkFallback: s.LinkFallback, openAccountLink: open } }],
      bindings: { ...s, ...words, ChoosePlanScreen, ACCOUNT_SWITCH_FAILED: 'Couldn’t sign out' },
    })
    mounted.push(m)
    const screen = () => m.tree().props
    return { m, order, screen }
  }

  test('the 15-minute re-check starts before the browser opens, and the button opens the step link', async () => {
    const c = choosePlan(noPlan(), async () => ({ opened: true }))
    expect(c.screen().waiting).toBe(false)
    await c.screen().onChoose(); await c.m.flush()
    expect(c.order).toEqual(['plan_opened', `open ${STEP}`])
    expect(c.screen().waiting).toBe(true)
    expect(c.screen().fallbackUrl).toBeNull()
  })

  test('when the browser does not open, the address shows and the re-check still runs', async () => {
    const c = choosePlan(noPlan(), async (url) => ({ opened: false, url }))
    await c.screen().onChoose(); await c.m.flush()
    expect(c.order[0]).toBe('plan_opened')
    expect(c.screen().fallbackUrl).toBe(STEP)
  })

  test('Sign out is a sign-out by choice, once; a failure is a toast', async () => {
    const ok = choosePlan(noPlan(), async () => ({ opened: true }))
    await ok.screen().onSignOut(); await ok.m.flush()
    expect(ok.order.filter((o) => o === 'sign_out')).toHaveLength(1)
    const failing = choosePlan(noPlan(), async () => ({ opened: true }), async () => ({ ok: false, reason: 'Could not stop the sync engine.', unsupported: false }))
    await failing.screen().onSignOut(); await failing.m.flush()
    expect(failing.m.toasts.map((t) => t.title)).toEqual(['Couldn’t sign out'])
  })
})

describe('ChoosePlanScreen and UpdateScreen', () => {
  function render(name: string, props: any) {
    const s = stubs()
    const m = mount('accountScreens.tsx', name, { backend: {}, props, bindings: { ...s, ...words } })
    mounted.push(m)
    return { m, s, texts: () => m.elements().filter((el) => el.type === 'button').map((el) => textOf(el.props.children)) }
  }
  const noop = () => {}

  test('choose a plan: the approved words, the waiting line after the click, the offline line in place of the button', () => {
    const before = render('ChoosePlanScreen', { offline: false, waiting: false, busy: false, fallbackUrl: null, onChoose: noop, onRetry: noop, onSignOut: noop })
    expect(before.texts()).toEqual([copy.CHOOSE_PLAN_BUTTON, copy.SIGN_OUT])
    expect(before.m.elements().some((el) => textOf(el.props.children) === copy.CHOOSE_PLAN_WAITING)).toBe(false)
    const after = render('ChoosePlanScreen', { offline: false, waiting: true, busy: false, fallbackUrl: STEP, onChoose: noop, onRetry: noop, onSignOut: noop })
    expect(after.m.elements().some((el) => textOf(el.props.children) === copy.CHOOSE_PLAN_WAITING)).toBe(true)
    expect(after.m.elements().find((el) => el.type === after.s.LinkFallback)!.props.url).toBe(STEP)
    const offline = render('ChoosePlanScreen', { offline: true, waiting: false, busy: false, fallbackUrl: null, onChoose: noop, onRetry: noop, onSignOut: noop })
    expect(offline.texts()).toEqual([copy.SIGN_OUT])
    expect(offline.m.elements().some((el) => el.type === offline.s.OfflineLine)).toBe(true)
  })

  test('update: Update now, or the offline line', () => {
    expect(render('UpdateScreen', { offline: false, busy: false, onUpdate: noop, onRetry: noop }).texts()).toEqual([copy.UPDATE_NOW])
    const offline = render('UpdateScreen', { offline: true, busy: false, onUpdate: noop, onRetry: noop })
    expect(offline.texts()).toEqual([])
    expect(offline.m.elements().some((el) => el.type === offline.s.OfflineLine)).toBe(true)
  })
})

describe('AccountUpdate', () => {
  function update(kind: string) {
    const calls: string[] = []
    const UpdateScreen = (props: any) => ({ type: 'UpdateScreen', props })
    const m = mount('accountScreens.tsx', 'AccountUpdate', {
      backend: {},
      props: { view: { ...noPlan(), state: 'update_required', screen: 'update' } },
      bindings: {
        ...stubs(), UpdateScreen,
        desktopUpdateCheck: { check: async () => { calls.push('check') }, getSnapshot: () => ({ kind }) },
        command: async (name: string) => { calls.push(name); return { ok: true, value: null } },
        retryAccountView: async () => ({ ok: true, value: undefined }),
        commandUnavailableLabel: (name: string) => name,
      },
    })
    mounted.push(m)
    const screen = () => m.elements().find((el) => el.type === UpdateScreen)!.props
    return { m, calls, screen }
  }

  test('Update now checks, then installs what it found, as Settings does', async () => {
    const u = update('update_available')
    await u.screen().onUpdate(); await u.m.flush()
    expect(u.calls).toEqual(['check', 'install_update'])
  })

  test('nothing to install: only the check runs, and its result shows as in Settings', async () => {
    const u = update('up_to_date')
    await u.screen().onUpdate(); await u.m.flush()
    expect(u.calls).toEqual(['check'])
  })
})

describe('BusyScreen', () => {
  test('Checking shows the busy indicator and no words (C-R11)', () => {
    const m = mount('accountScreens.tsx', 'BusyScreen', { backend: {}, bindings: {} })
    mounted.push(m)
    expect(textOf(m.tree())).toBe('')
    expect(m.tree().props['aria-busy']).toBe('true')
  })
})

describe('accountWindowTitle', () => {
  test('the title bar follows the heading', async () => {
    const { accountWindowTitle } = await import('../src/accountScreens')
    expect(accountWindowTitle('sign_in')).toBe(copy.SIGN_IN_TITLE)
    expect(accountWindowTitle('sign_in_again')).toBe(copy.SIGN_IN_AGAIN)
    expect(accountWindowTitle('update')).toBe(copy.UPDATE_TITLE)
    expect(accountWindowTitle('choose_plan')).toBe(copy.CHOOSE_PLAN_TITLE)
    for (const screen of ['busy', 'recovery_phrase', 'nothing', 'keychain_unlock'] as const) expect(accountWindowTitle(screen)).toBe(copy.SIGN_IN_TITLE)
  })
})
```

`tests/macAccountWindow.test.tsx`:

```tsx
/**
 * Spec 2026-10-07 §7 (macOS): the account window renders the view's screen and nothing else. It also carries spec A's
 * R8 rules for this window: the same account never signs out; another account sees the switch warning with its count,
 * and nothing is cleared before the click (the warning's own click is spec A's `AccountSwitchStep` test).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { AccountScreen, AccountView, AccountViewLoad } from '../src/accountView'
import { mount, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const LINKS = { create_account: 'https://app.beebeeb.io/signup', billing: 'https://app.beebeeb.io/billing', support: 'https://beebeeb.io/support', step: 'https://app.beebeeb.io/billing' }
const viewOn = (screen: AccountScreen, fields: Partial<AccountView> = {}): AccountView =>
  ({ state: 'signed_out', session: 'none', offline: false, screen, busy: screen === 'busy', links: LINKS, notice: null, ...fields }) as AccountView

function windowOn(initial: AccountViewLoad) {
  const world = { load: initial }
  const names = ['SignInMethods', 'AccountUpdate', 'UnlockStep', 'AccountChoosePlan', 'BusyScreen', 'AfterSignInFinder', 'AccountSwitchStep']
  const stubs: Record<string, (props: any) => null> = {}
  for (const name of names) stubs[name] = () => null
  const titles: string[] = []
  const hidden: number[] = []
  const m = mount('Onboarding.tsx', 'MacAccountWindow', {
    backend: { show_settings_on_account: () => null, clear_session: () => null },
    bindings: {
      ...stubs,
      useAccountView: () => world.load,
      accountWindowTitle: (screen: string) => `title:${screen}`,
      command: async (name: string) => { (m as any).calls.push({ name }); return { ok: true, value: null } },
      getCurrentWindow: () => ({ setTitle: async (t: string) => { titles.push(t) }, hide: async () => { hidden.push(1) }, close: async () => {} }),
    },
  })
  mounted.push(m)
  const shown = () => names.filter((name) => m.elements().some((el) => el.type === stubs[name]))
  const find = (name: string) => m.elements().find((el) => el.type === stubs[name])
  const setView = async (view: AccountView) => { world.load = { status: 'ready', view }; m.render(); await m.flush() }
  return { m, shown, find, setView, titles, hidden }
}

describe('MacAccountWindow', () => {
  test('every screen of the view routes to its own screen', async () => {
    const cases: Array<[AccountScreen, string]> = [
      ['sign_in', 'SignInMethods'], ['sign_in_again', 'SignInMethods'], ['update', 'AccountUpdate'], ['recovery_phrase', 'UnlockStep'],
      ['choose_plan', 'AccountChoosePlan'], ['busy', 'BusyScreen'], ['keychain_unlock', 'BusyScreen'], ['nothing', 'AfterSignInFinder'],
    ]
    for (const [screen, expected] of cases) {
      const w = windowOn({ status: 'ready', view: viewOn(screen) })
      await w.m.flush()
      expect({ screen, shown: w.shown() }).toEqual({ screen, shown: [expected] })
    }
    const again = windowOn({ status: 'ready', view: viewOn('sign_in_again') })
    expect(again.find('SignInMethods')!.props.again).toBe(true)
    expect(windowOn({ status: 'loading' }).shown()).toEqual(['BusyScreen'])
    const failed = windowOn({ status: 'failed' })
    expect(failed.shown()).toEqual(['SignInMethods'])
    expect(failed.find('SignInMethods')!.props.view).toBeNull()
  })

  /** C-S1: whether a sign-in brought its keys is decided in Rust (Task 12's outcome, Task 5's
   *  `a_browser_sign_in_with_its_keys_is_never_the_recovery_phrase`). This window renders only the view, so what it
   *  must do is wait: busy from the sign-in until the view moves on, then the view's own screen. */
  test('after a sign-in the window is busy until the account view moves on, then shows what it says', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('sign_in') })
    w.find('SignInMethods')!.props.onSettled({ kind: 'fresh', vaultUnlocked: true }); w.m.render()
    expect(w.shown()).toEqual(['BusyScreen'])
    await w.setView(viewOn('busy', { state: 'checking', session: 'valid' }))
    expect(w.shown()).toEqual(['BusyScreen'])
    await w.setView(viewOn('nothing', { state: 'ready', session: 'valid' }))
    expect(w.shown()).toEqual(['AfterSignInFinder'])
  })

  test('a password sign-in without a key on this Mac shows the recovery phrase once the view says so', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('sign_in') })
    w.find('SignInMethods')!.props.onSettled({ kind: 'fresh', vaultUnlocked: false }); w.m.render()
    await w.setView(viewOn('recovery_phrase', { state: 'locked', session: 'valid' }))
    expect(w.shown()).toEqual(['UnlockStep'])
  })

  test('another account sees the switch warning with its count; nothing is cleared, and Cancel goes back', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('sign_in_again', { state: 'session_ended', session: 'ended' }) })
    w.find('SignInMethods')!.props.onSettled({ kind: 'account_mismatch', pendingChanges: 3 }); w.m.render()
    expect(w.find('AccountSwitchStep')!.props.pendingChanges).toBe(3)
    expect(w.m.calls.map((c) => c.name)).not.toContain('clear_session')
    w.find('AccountSwitchStep')!.props.onCancel(); w.m.render()
    expect(w.shown()).toEqual(['SignInMethods'])
    expect(w.m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('the same account signing in again never signs out', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('sign_in_again', { state: 'session_ended', session: 'ended' }) })
    w.find('SignInMethods')!.props.onSettled({ kind: 'reauthenticated', vaultUnlocked: true }); w.m.render()
    await w.setView(viewOn('nothing', { state: 'ready', session: 'valid' }))
    expect(w.m.calls.map((c) => c.name)).not.toContain('clear_session')
  })

  test('the window title follows the heading', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('update', { state: 'update_required', session: 'valid' }) })
    await w.m.flush()
    await w.setView(viewOn('choose_plan', { state: 'no_plan', session: 'valid' }))
    expect(w.titles).toEqual(['title:update', 'title:choose_plan'])
  })

  test('the Keychain unlock hands over to Settings on its Account tab and steps aside', async () => {
    const w = windowOn({ status: 'ready', view: viewOn('keychain_unlock', { state: 'locked', session: 'valid' }) })
    await w.m.flush()
    expect(w.m.calls.map((c) => c.name)).toContain('show_settings_on_account')
    expect(w.hidden).toEqual([1])
  })
})

describe('AfterSignInFinder (§7.6)', () => {
  function finderWith(presentation: any) {
    const closed: number[] = []
    const run: string[] = []
    const m = mount('Onboarding.tsx', 'AfterSignInFinder', {
      backend: {},
      bindings: {
        useFinderSetup: () => ({ presentation, run: async (action: string) => { run.push(action) }, retry: async () => { run.push('retry') } }),
        getCurrentWindow: () => ({ close: async () => { closed.push(1) } }),
        AccountCard: (props: any) => props.children, BusyScreen: () => null, FINDER_SETUP_TITLE: 'Beebeeb in Finder',
      },
    })
    mounted.push(m)
    return { m, closed, run }
  }

  test('Adding shows spec A’s line; Ready closes the window', async () => {
    const adding = finderWith({ kind: 'adding', line: 'Adding Beebeeb to Finder…' })
    await adding.m.flush()
    expect(adding.m.elements().some((el) => el.props.children === 'Adding Beebeeb to Finder…')).toBe(true)
    expect(adding.closed).toEqual([])
    const ready = finderWith({ kind: 'ready', line: 'Your vault appears under Locations in Finder.' })
    await ready.m.flush()
    expect(ready.closed).toEqual([1])
  })

  test('a failed first add shows spec A’s one sentence and one action', async () => {
    const failed = finderWith({ kind: 'notice', tone: 'alert', sentence: 'macOS hasn’t finished loading Beebeeb’s Finder extension.', actionLabel: 'Try again', action: 'try_again' })
    await failed.m.click('Try again')
    expect(failed.run).toEqual(['try_again'])
  })
})

describe('the account window may set its title and close itself (C-W8, §7.6; plan review I2)', () => {
  test('one capability grants exactly those two to the onboarding window', () => {
    const capability = JSON.parse(readFileSync(new URL('../src-tauri/capabilities/account-window.json', import.meta.url), 'utf8'))
    expect(capability.windows).toEqual(['onboarding'])
    expect([...capability.permissions].sort()).toEqual(['core:window:allow-close', 'core:window:allow-set-title'])
    const source = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
    expect(source).toContain('getCurrentWindow().setTitle(')
    expect(source).toContain('getCurrentWindow().close()')
  })
})

describe('macOS has no Finder step, no pinning or ready steps, and no rail (§11)', () => {
  test('the step is gone, and Linux keeps its steps', () => {
    const source = readFileSync(new URL('../src/Onboarding.tsx', import.meta.url), 'utf8')
    expect(source).not.toContain('function MacFinderStep(')
    expect(source).not.toContain("platform === 'macos' ? (")
    for (const linux of ['function FinderInstallStep(', 'function PinningStep(', 'function ReadyStep(']) expect(source).toContain(linux)
    const main = readFileSync(new URL('../src/main.tsx', import.meta.url), 'utf8')
    expect(main).toContain("which === 'onboarding' && platform === 'macos'")
    expect(main).toContain('<MacAccountWindow />')
  })
})
```

(`m.calls` is the harness's command log; the `command` binding above pushes to it so the bound function and the harness agree.)

Run: `bun test tests/accountScreens.test.tsx tests/macAccountWindow.test.tsx > $EVID/t17-red.log 2>&1; echo "rc=$?"`. Expected: `rc=1` (missing components). Paste.

- [ ] **Step 2: The screens in `accountScreens.tsx`**

Add:

```tsx
/** §7.3: "Update Beebeeb". Offline, the offline line and "Try again" replace "Update now". */
export function UpdateScreen({ offline, busy, onUpdate, onRetry }: { offline: boolean; busy: boolean; onUpdate: () => void; onRetry: () => void }) {
  return (
    <AccountCard title={UPDATE_TITLE} copy={UPDATE_BODY}>
      {offline ? (
        <OfflineLine busy={busy} onRetry={onRetry} />
      ) : (
        <button className="button amber" disabled={busy} onClick={onUpdate}>
          {UPDATE_NOW}
        </button>
      )}
    </AccountCard>
  )
}

/** §7.4: "Choose a plan", one amber button, the waiting line after the click, and a quiet "Sign out". */
export function ChoosePlanScreen({
  offline, waiting, busy, fallbackUrl, onChoose, onRetry, onSignOut,
}: {
  offline: boolean; waiting: boolean; busy: boolean; fallbackUrl: string | null; onChoose: () => void; onRetry: () => void; onSignOut: () => void
}) {
  return (
    <AccountCard title={CHOOSE_PLAN_TITLE} copy={CHOOSE_PLAN_BODY}>
      {offline ? (
        <OfflineLine busy={busy} onRetry={onRetry} />
      ) : (
        <button className="button amber" disabled={busy} onClick={onChoose}>
          {CHOOSE_PLAN_BUTTON}
        </button>
      )}
      {waiting ? <p className="page-copy" role="status">{CHOOSE_PLAN_WAITING}</p> : null}
      {fallbackUrl ? <LinkFallback url={fallbackUrl} /> : null}
      <button className="text-link quiet" disabled={busy} onClick={onSignOut}>
        {SIGN_OUT}
      </button>
    </AccountCard>
  )
}

/** Checking (C-R11): the window's busy indicator, with no new words. */
export function BusyScreen() {
  return (
    <section className="auth-card account-card account-busy" aria-busy="true">
      <div className="account-spinner" aria-hidden="true" />
    </section>
  )
}

/** §7.3: "Update now" runs today's updater, `check_for_updates_now` then `install_update`; its results show as in
 *  Settings (the manual-check toasts, and Settings' own "Update install failed"). */
export function AccountUpdate({ view }: { view: AccountView }) {
  const { showToast } = useToast()
  const [busy, setBusy] = useState(false)
  const update = async () => {
    setBusy(true)
    await desktopUpdateCheck.check()
    if (desktopUpdateCheck.getSnapshot().kind === 'update_available') {
      const result = await command<void>('install_update')
      if (!result.ok) {
        showToast({ variant: 'error', title: 'Update install failed', message: result.unsupported ? commandUnavailableLabel('install_update') : result.reason })
      }
    }
    setBusy(false)
  }
  const retry = async () => {
    setBusy(true)
    await retryAccountView()
    setBusy(false)
  }
  return (
    <>
      <UpdateScreen offline={view.offline} busy={busy} onUpdate={() => void update()} onRetry={() => void retry()} />
      <ManualUpdateFeedback />
    </>
  )
}

/** §7.4: the click starts the 15-minute re-check first, whether or not the browser opens (plan "Spec issues" 17),
 *  then opens the account view's step link. "Sign out" is spec A's sign-out by choice. */
export function AccountChoosePlan({
  view,
  open = openAccountLink,
  planOpened = accountViewPlanOpened,
  retry = retryAccountView,
  signOut = () => command<void>('clear_session'),
}: {
  view: AccountView
  open?: (url: string) => Promise<LinkOpen>
  planOpened?: () => Promise<CommandResult<void>>
  retry?: () => Promise<CommandResult<void>>
  signOut?: () => Promise<CommandResult<void>>
}) {
  const { showToast } = useToast()
  const [waiting, setWaiting] = useState(false)
  const [busy, setBusy] = useState(false)
  // §10 at this link too (plan review I7): the one opener every link site uses.
  const { openLink, failedUrl } = useLinkOpener(open)
  const choose = async () => {
    await planOpened()
    setWaiting(true)
    await openLink(view.links.step)
  }
  const signOutNow = async () => {
    setBusy(true)
    const result = await signOut()
    setBusy(false)
    if (!result.ok) showToast({ variant: 'error', title: ACCOUNT_SWITCH_FAILED, message: result.reason })
  }
  const retryNow = async () => {
    setBusy(true)
    await retry()
    setBusy(false)
  }
  return (
    <ChoosePlanScreen
      offline={view.offline}
      waiting={waiting}
      busy={busy}
      fallbackUrl={failedUrl}
      onChoose={() => void choose()}
      onRetry={() => void retryNow()}
      onSignOut={() => void signOutNow()}
    />
  )
}

/** C-W8: the title bar follows the heading of the screen it shows. */
export function accountWindowTitle(screen: AccountScreen): string {
  switch (screen) {
    case 'update':
      return UPDATE_TITLE
    case 'choose_plan':
      return CHOOSE_PLAN_TITLE
    case 'sign_in_again':
      return SIGN_IN_AGAIN
    default:
      return SIGN_IN_TITLE
  }
}
```

Imports to add: `AccountScreen`, `AccountView`, `accountViewPlanOpened`, `retryAccountView` from `./accountView`; the update, choose-a-plan, sign-in and sign-out words from `./accountViewCopy`; `commandUnavailableLabel` from `./desktopApi`; `desktopUpdateCheck` from `./windows/manualUpdateCheck`; the default export `ManualUpdateFeedback` (`import ManualUpdateFeedback from './ManualUpdateFeedback'`); `useToast` from `./windows/ui`; `ACCOUNT_SWITCH_FAILED` from `./accountSwitchCopy`. In the step 1 tests, `AccountChoosePlan`'s `ChoosePlanScreen` and `AccountUpdate`'s `UpdateScreen` are stubbed so their props can be driven directly.

- [ ] **Step 3: The window in `Onboarding.tsx`**

Delete `function MacFinderStep(…) { … }` (spec A §10 handed its deletion to this spec; §11). In `OnboardingView` (now Linux and an unknown platform only), remove the macOS branches: the `rail` mapping that swapped in `FINDER_RAIL_TITLE`/`FINDER_RAIL_DETAIL` (render `STEPS` as written), the `if (resolved === 'macos') { … loadFinderSetup … }` fast-forward, and the `platform === 'macos' ? <MacFinderStep …/> : <FinderInstallStep …/>` choice (always `FinderInstallStep`). Give it the view for its sign-in step:

```tsx
  const accountLoad = useAccountView()
  const accountView = accountLoad.status === 'ready' ? accountLoad.view : null
  …
        {step === 'signin' && <SignInMethods view={accountView} again={mode === 'reauth'} onSettled={afterSignIn} />}
```

Add:

```tsx
/**
 * Spec 2026-10-07 §7 (macOS): the account window. It renders the account view's screen and decides nothing itself. Its
 * only local state: the switch warning after a sign-in turned out to be another account (spec A's R8), and the moment
 * between a sign-in and the view moving on (shown as busy, so the sign-in form never flashes back).
 */
export function MacAccountWindow() {
  const load = useAccountView()
  const [pendingSwitch, setPendingSwitch] = useState<{ count: number } | null>(null)
  const [settlingFrom, setSettlingFrom] = useState<AccountScreen | null>(null)
  const view = load.status === 'ready' ? load.view : null
  const screen: AccountScreen = view?.screen ?? 'busy'

  useEffect(() => {
    if (settlingFrom !== null && screen !== settlingFrom) setSettlingFrom(null)
  }, [screen, settlingFrom])

  useEffect(() => {
    void getCurrentWindow().setTitle(accountWindowTitle(screen))
  }, [screen])

  useEffect(() => {
    if (screen !== 'keychain_unlock') return
    // Plan "Spec issues" 13: the Keychain unlock is Settings' Account tab (C-W7). Hand over and step aside.
    void command<void>('show_settings_on_account').then(() => getCurrentWindow().hide())
  }, [screen])

  const afterSignIn = (settled: SignInSettled) => {
    if (settled.kind === 'account_mismatch') {
      setPendingSwitch({ count: settled.pendingChanges })
      return
    }
    // Every other outcome is the view's to show next: Checking, Locked (the recovery phrase) or Ready.
    setSettlingFrom(screen)
  }

  let body: ReactNode
  if (pendingSwitch) {
    body = <AccountSwitchStep pendingChanges={pendingSwitch.count} onSwitched={() => setPendingSwitch(null)} onCancel={() => setPendingSwitch(null)} />
  } else if (load.status === 'failed') {
    body = <SignInMethods view={null} again={false} onSettled={afterSignIn} />
  } else if (!view || settlingFrom !== null) {
    body = <BusyScreen />
  } else {
    switch (view.screen) {
      case 'sign_in':
        body = <SignInMethods view={view} again={false} onSettled={afterSignIn} />
        break
      case 'sign_in_again':
        body = <SignInMethods view={view} again onSettled={afterSignIn} />
        break
      case 'update':
        body = <AccountUpdate view={view} />
        break
      case 'recovery_phrase':
        body = <UnlockStep onDone={() => setSettlingFrom('recovery_phrase')} />
        break
      case 'choose_plan':
        body = <AccountChoosePlan view={view} />
        break
      case 'nothing':
        body = <AfterSignInFinder />
        break
      case 'keychain_unlock':
      case 'busy':
        body = <BusyScreen />
        break
    }
  }
  return (
    <div className="account-window">
      <main className="account-window-main">{body}</main>
    </div>
  )
}

/** §7.6: after sign-in on macOS, spec A's "Adding Beebeeb to Finder…", closing by itself on Ready. On Failed or
 *  UserDisabled, spec A's one sentence and one action, so a failed first add is never unseen until spec B. */
function AfterSignInFinder() {
  const finder = useFinderSetup()
  const presentation = finder.presentation
  useEffect(() => {
    if (presentation.kind === 'ready') void getCurrentWindow().close()
  }, [presentation.kind])
  if (presentation.kind === 'notice' || presentation.kind === 'unavailable') {
    const alert = presentation.kind === 'unavailable' || presentation.tone === 'alert'
    const sentence = presentation.kind === 'notice' ? presentation.sentence : presentation.line
    const act = presentation.kind === 'notice' ? () => finder.run(presentation.action) : () => finder.retry()
    return (
      <AccountCard title={FINDER_SETUP_TITLE}>
        <div className={alert ? 'notice error' : 'notice'} role={alert ? 'alert' : 'status'} data-error-surface={alert ? 'finder-setup' : undefined}>
          <div>{sentence}</div>
          <div className="button-row" style={{ marginTop: 10 }}>
            <button className="button" onClick={() => void act()}>
              {presentation.actionLabel}
            </button>
          </div>
        </div>
      </AccountCard>
    )
  }
  if (presentation.kind === 'adding') {
    return (
      <AccountCard title={FINDER_SETUP_TITLE}>
        <div className="panel mono">{presentation.line}</div>
      </AccountCard>
    )
  }
  return <BusyScreen />
}
```

If T11-M5 ran in Task 16, `pendingSwitch` is `{ count: number | null }` (the type follows `SignInSettled`).

Imports: `useAccountView`, `AccountScreen` from `./accountView`; `AccountUpdate`, `AccountChoosePlan`, `BusyScreen`, `accountWindowTitle` from `./accountScreens`; keep `FINDER_SETUP_TITLE`; drop `FINDER_RAIL_TITLE`, `FINDER_RAIL_DETAIL` and `loadFinderSetup` if nothing else uses them (the constants stay in `finderSetupCopy.ts`, which spec A's tests pin).

- [ ] **Step 4: Route it in `main.tsx` and style it**

In `main.tsx`, before the existing `which === 'onboarding'` branches:

```tsx
} else if (which === 'onboarding' && platform === 'macos') {
  // Spec 2026-10-07 C-W8: the macOS account window (Rust tags its URL, plan Task 13).
  component = <CapabilityGate route="onboarding"><MacAccountWindow /></CapabilityGate>
```

and `import Onboarding, { MacAccountWindow } from './Onboarding'`. Append to `src/design.css`:

```css
/* Spec 2026-10-07 C-W8: the macOS account window, centred, no step rail. */
.account-window { min-height: 100vh; display: grid; place-items: center; padding: 24px; background: var(--paper); }
.account-window-main { width: min(460px, 100%); }
.account-busy { display: grid; place-items: center; min-height: 240px; }
.account-spinner { width: 22px; height: 22px; border-radius: 50%; border: 2px solid var(--line-2); border-top-color: var(--ink-2); animation: account-spin 0.9s linear infinite; }
@keyframes account-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .account-spinner { animation: none; } }
```

(the spinner is neutral: amber is for primary actions and encryption state only).

The window calls `getCurrentWindow().setTitle(…)` (C-W8) and `getCurrentWindow().close()` (§7.6), and `core:window:default` grants neither, so without a capability both are refused silently (the calls are `void`ed). Create `src-tauri/capabilities/account-window.json` (add `src-tauri/tauri.conf.json` to Step 8's two pathspec lists if you change it below):

```json
{
  "identifier": "account-window",
  "description": "Spec 2026-10-07 C-W8 and §7.6: the account window sets its title to its heading and closes itself once Finder is ready.",
  "windows": ["onboarding"],
  "permissions": ["core:window:allow-set-title", "core:window:allow-close"]
}
```

Copy `default.json`'s `$schema` line into it if `default.json` has one. If `tauri.conf.json` lists capabilities by identifier (`app.security.capabilities`), add `"account-window"` there. Then let tauri-build check the permission names: `cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked > $EVID/t17-cargo-check.log 2>&1; echo "rc=$?"`, expect `rc=0` (an unknown permission fails the build).

- [ ] **Step 5: Run the new tests**

`bun test tests/accountScreens.test.tsx tests/macAccountWindow.test.tsx > $EVID/t17-green.log 2>&1; echo "rc=$?"`. Expected: `rc=0`, `20 pass`, `0 fail` (`accountScreens` 9, `macAccountWindow` 11).

- [ ] **Step 6: Bring spec A's step-flow tests in line**

- `tests/onboardingFinderStep.test.tsx`: the cases that mount `MacFinderStep`, or `OnboardingView` with `desktop_platform: () => 'macos'` expecting the macOS Finder step, test the step this spec deletes (§11; spec A §10 handed the deletion here). Delete those cases. Keep every `FinderInstallStep` (Windows/Linux) case, the Linux routing cases and Task 16's two Linux step-flow tests. The behaviour they pinned now lives in `tests/macAccountWindow.test.tsx` (`AfterSignInFinder`: Adding, Ready closes, a failed add shows one sentence and one action).
- `OnboardingView` now calls `useAccountView()`, so every mount of it needs that binding (the harness only sees what it is given). In `tests/onboardingFinderStep.test.tsx`'s `openOnboarding`, add to its `bindings`:

  ```ts
      // Task 17: OnboardingView reads the account view for its sign-in step; these tests drive the steps, not the view.
      useAccountView: () => ({ status: 'loading' }),
  ```

  and add the same line to the `bindings` of `tests/reauthInPlace.test.tsx`'s `mount('Onboarding.tsx', 'OnboardingView', …)`.
- `tests/reauthInPlace.test.tsx`: `OnboardingView` is the Linux step flow now. Change its `desktop_platform` answer from `'macos'` to `'linux'` and its `useCapabilities` to `host_os: 'linux'`; the R8 assertions stay (Linux has R8, spec A). The macOS window's R8 rules are in `tests/macAccountWindow.test.tsx`. `AccountSwitchStep`'s own tests (exactly one `clear_session`, after the click) stay as they are.

Record in the task Notes the names of every deleted test and the test that now covers its behaviour (or "covered by: none — the behaviour was removed by §11").

Run: `bun test > $EVID/t17-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t17-all.log`. Expected: `0 fail`, and the pass count is `B_bun` + 55 minus the spec A tests this step deleted (their number is in the Notes; Global Constraints).

- [ ] **Step 7: Mutation-check**

1. In `MacAccountWindow.afterSignIn`, delete `setSettlingFrom(screen)`. Expected: `after a sign-in the window is busy until the account view moves on…` (it shows `SignInMethods`). Revert.
2. In `AccountChoosePlan.choose`, call `openLink` before `planOpened`. Expected: `the 15-minute re-check starts before the browser opens…`. Revert.
3. Remove the `'keychain_unlock'` effect. Expected: `the Keychain unlock hands over…`. Revert.
4. Add `<p className="page-copy">Checking…</p>` inside `BusyScreen`. Expected: `Checking shows the busy indicator and no words`. Revert.
5. Delete `"core:window:allow-close"` from `account-window.json`. Expected: `one capability grants exactly those two to the onboarding window`. Revert.
6. In `AccountChoosePlan`, pass `fallbackUrl={null}`. Expected: `when the browser does not open, the address shows and the re-check still runs`. Revert.

- [ ] **Step 8: Commit**

```bash
cd $WT && bunx tsc --noEmit > $EVID/t17-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t17-eslint.log 2>&1; echo "rc=$?"
git add src/Onboarding.tsx src/accountScreens.tsx src/main.tsx src/design.css src-tauri/capabilities/account-window.json tests/accountScreens.test.tsx tests/macAccountWindow.test.tsx tests/onboardingFinderStep.test.tsx tests/reauthInPlace.test.tsx
git commit -m "desktop: on macOS the first-run window is the account window: sign in, update, choose a plan, then Finder" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/Onboarding.tsx src/accountScreens.tsx src/main.tsx src/design.css src-tauri/capabilities/account-window.json tests/accountScreens.test.tsx tests/macAccountWindow.test.tsx tests/onboardingFinderStep.test.tsx tests/reauthInPlace.test.tsx
git show --stat HEAD
```

---

## Task 18: The Ready notices in Settings, and one source for every billing link (spec §5.1, §7.5)

**Lane T.**

**Files:**
- Modify: `src/accountScreens.tsx` (`AccountNoticeLine`), `src/accountLinks.ts` (`planButton`)
- Modify: `src/MacSettings.tsx` (Account tab), `src/windows/views/AccountView.tsx` (notice row, the frozen row, billing link), `src/WindowsApp.tsx` (`openUpgrade`), `src/pages/Account.tsx`, `src/pages/VersionCenter.tsx`, `src/desktopApi.ts` (delete `BILLING_URL`), `src/macSettingsModel.ts` (`HELP_URL`)
- Test: `tests/accountNotices.test.tsx` (new); `tests/accountLinks.test.ts` (the scan); `tests/macSettingsTabs.test.tsx`, `tests/macosAccountPlanUi.test.ts`, `tests/macSettingsModel.test.ts` (follow the new source)

**Interfaces:**
- Consumes: Task 15 (`useAccountView`, `AccountNotice`, `noticeLine`, `NOTICE_LINK_LABEL`, `linkFor`, `openAccountLink`, `BUILT_IN_LINKS`), Task 16 (`LinkFallback`, `useLinkOpener`).
- Produces: `src/accountScreens.tsx` `AccountNoticeLine({ notice })`; `src/accountLinks.ts` `planButton(view: AccountView | null, todayLabel: string): { label: string; url: string }`.

- [ ] **Step 1: Write the failing tests**

`tests/accountNotices.test.tsx`:

```tsx
/** Spec 2026-10-07 §7.5: the notice line, and its link replacing the plan button. */
import { afterEach, describe, expect, test } from 'bun:test'
import { BUILT_IN_LINKS, planButton } from '../src/accountLinks'
import { noticeLine, NOTICE_LINK_LABEL } from '../src/accountViewCopy'
import type { AccountNotice, AccountView } from '../src/accountView'
import { mount, textOf, type Mounted } from './fixtures/componentHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const LINKS = { create_account: 'https://app.beebeeb.io/signup', billing: 'https://app.beebeeb.io/billing?server', support: 'https://beebeeb.io/support?server', step: 'https://app.beebeeb.io/billing?server' }
const notice = (fields: Partial<AccountNotice>): AccountNotice =>
  ({ kind: 'trial', server_line: null, trial_ends: null, read_only_since: null, data_deletion_at: null, link: null, url: null, ...fields })
const ready = (n: AccountNotice | null): AccountView =>
  ({ state: 'ready', session: 'valid', offline: false, screen: 'nothing', busy: false, links: LINKS, notice: n }) as AccountView

describe('planButton', () => {
  test('a notice’s link replaces the plan button', () => {
    expect(planButton(ready(notice({ kind: 'frozen', link: 'contact_support', url: LINKS.support })), 'Manage plan'))
      .toEqual({ label: NOTICE_LINK_LABEL.contact_support, url: LINKS.support })
    expect(planButton(ready(notice({ kind: 'payment_failed', link: 'update_payment_details', url: LINKS.billing })), 'Manage plan'))
      .toEqual({ label: 'Update payment details', url: LINKS.billing })
  })

  test('with no notice, or one without a link, the button stays and opens the resolved web billing', () => {
    expect(planButton(ready(null), 'Upgrade')).toEqual({ label: 'Upgrade', url: LINKS.billing })
    expect(planButton(ready(notice({ kind: 'read_only_other' })), 'Manage plan')).toEqual({ label: 'Manage plan', url: LINKS.billing })
    expect(planButton(null, 'Manage plan')).toEqual({ label: 'Manage plan', url: BUILT_IN_LINKS.billing })
  })
})

describe('AccountNoticeLine', () => {
  function line(n: AccountNotice) {
    const m = mount('accountScreens.tsx', 'AccountNoticeLine', { backend: {}, props: { notice: n }, bindings: { noticeLine } })
    mounted.push(m)
    return m.tree() ? textOf(m.tree().props.children) : null
  }

  test('renders the notice line, the server’s words first', () => {
    expect(line(notice({ kind: 'trial', trial_ends: '18 Oct' }))).toBe('Free trial until 18 Oct.')
    expect(line(notice({ kind: 'read_only', server_line: 'Server words.' }))).toBe('Server words.')
  })

  test('nothing honest to say renders nothing', () => {
    expect(line(notice({ kind: 'trial' }))).toBeNull()
  })
})

/** §10 "Browser won't open" at every place an account link opens (plan review I7): each site opens through
 *  `useLinkOpener` and renders its fallback (the address and "Copy link"), and none opens an account link directly. */
describe('every account link shows its address when the browser does not open', () => {
  test('each link site uses the one opener and renders its fallback', () => {
    const sites = ['accountScreens.tsx', 'MacSettings.tsx', 'windows/views/AccountView.tsx', 'WindowsApp.tsx', 'pages/Account.tsx', 'pages/VersionCenter.tsx']
    for (const site of sites) {
      const text = readFileSync(new URL(`../src/${site}`, import.meta.url), 'utf8')
      const direct = ['openUrl(linkFor(', 'openUrl(BILLING', 'openAccountLink(notice.url', 'openAccountLink(url', 'openUrl(button.url'].filter((call) => text.includes(call))
      expect({ site, opener: text.includes('useLinkOpener('), fallback: /\{fallback\}|fallback=\{fallback\}|fallbackUrl=\{failedUrl\}/.test(text), direct })
        .toEqual({ site, opener: true, fallback: true, direct: [] })
    }
  })
})
```

(Add `import { readFileSync } from 'node:fs'` to the file's imports.)

In `tests/accountLinks.test.ts` add:

```ts
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

  test('no file in src/ but accountLinks.ts holds a built-in address (§5.1)', () => {
    const offenders: string[] = []
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name)
        if (entry.isDirectory()) walk(path)
        else if (/\.(ts|tsx)$/.test(entry.name) && !path.endsWith('accountLinks.ts')) {
          const text = readFileSync(path, 'utf8')
          for (const url of Object.values(BUILT_IN_LINKS)) if (text.includes(url)) offenders.push(`${path}: ${url}`)
        }
      }
    }
    const src = fileURLToPath(new URL('../src', import.meta.url))
    walk(src)
    expect(offenders).toEqual([])
    expect(readdirSync(src).length).toBeGreaterThan(20)
  })
```

Run: `bun test tests/accountNotices.test.tsx tests/accountLinks.test.ts > $EVID/t18-red.log 2>&1; echo "rc=$?"`. Expected: `rc=1`: `planButton`/`AccountNoticeLine` missing, and the scan lists `desktopApi.ts`, `macSettingsModel.ts`, `WindowsApp.tsx`, `windows/views/AccountView.tsx`. Paste the offender list.

- [ ] **Step 2: Implement `planButton` and `AccountNoticeLine`**

`src/accountLinks.ts`:

```ts
import { NOTICE_LINK_LABEL } from './accountViewCopy'

/** §7.5: while a notice with a link shows, its link replaces the Account tab's plan button; otherwise the button keeps
 *  today's label and opens the resolved web-billing link. */
export function planButton(view: AccountView | null, todayLabel: string): { label: string; url: string } {
  const notice = view?.notice
  if (notice?.link && notice.url) return { label: NOTICE_LINK_LABEL[notice.link], url: notice.url }
  return { label: todayLabel, url: linkFor(view, 'billing') }
}
```

`src/accountScreens.tsx`:

```tsx
/** §7.5: the one-line Ready notice (Settings' Account tab on macOS, the Account view on Windows). */
export function AccountNoticeLine({ notice }: { notice: AccountNotice }) {
  const line = noticeLine(notice)
  if (!line) return null
  return (
    <div className="account-notice" role="status">
      {line}
    </div>
  )
}
```

- [ ] **Step 3: Replace every hard-coded address**

- `src/desktopApi.ts`: delete `export const BILLING_URL = …` and its comment.
- `src/macSettingsModel.ts`: `export const HELP_URL = BUILT_IN_LINKS.support` (import `BUILT_IN_LINKS` from `./accountLinks`).
Every link below opens through Task 16's `useLinkOpener()` and renders its `fallback` right under the control that opened it (plan review I7: §10's "Browser won't open" holds at every site, not only the three the screens already had).

- `src/MacSettings.tsx`, `AccountTab`: add

```tsx
  const accountLoad = useAccountView()
  const accountView = accountLoad.status === 'ready' ? accountLoad.view : null
  const { openLink, fallback } = useLinkOpener()
```

  under the plan text (`{planText ? … : null}`): `{accountView?.notice ? <AccountNoticeLine notice={accountView.notice} /> : null}`; replace the plan button with

```tsx
          {unlocked ? (() => {
            const button = planButton(accountView, plan?.plan.toLowerCase() === 'free' ? 'Upgrade' : 'Manage plan')
            return <Btn onClick={() => void openLink(button.url)}>{button.label}</Btn>
          })() : null}
```

  and after the account row's group, `{fallback}`. Remove the `BILLING_URL` import.
- `src/pages/Account.tsx` and `src/pages/VersionCenter.tsx`: `useAccountView()` as above and `const { openLink, fallback } = useLinkOpener()`; `openUrl(BILLING_URL)` becomes `openLink(linkFor(accountView, 'billing'))`, with `{fallback}` under that button; remove the `BILLING_URL` import (other `openUrl` calls in these files, such as `WEB_APP_URL`, are not account links and stay).
- `src/WindowsApp.tsx`: `useAccountView()` and `const { openLink, fallback } = useLinkOpener()` near the top of the component; `const openUpgrade = () => { void openLink(linkFor(accountView, 'billing')) }`. `openUpgrade` goes to `<StorageWidget … onUpgrade={openUpgrade} />`: pass `fallback={fallback}` beside it and render `{fallback}` under the widget's upgrade control (one optional `fallback?: ReactNode` prop on `StorageWidget`, which is defined in the same file).
- `src/windows/views/AccountView.tsx`: delete `const BILLING_URL = …`; `useAccountView()` and `useLinkOpener()` in the component that renders the account card; the billing button calls `openLink(linkFor(accountView, 'billing'))`, with `{fallback}` under it. Render the notice (any kind) as a row in the same visual style as `FrozenRow`, and when the notice is `frozen` it replaces `FrozenRow` (§7.5):

```tsx
function NoticeRow({ notice }: { notice: AccountNotice }) {
  const { openLink, fallback } = useLinkOpener()
  const line = noticeLine(notice)
  if (!line) return null
  return (
    <div style={{ display: 'flex', alignItems: 'flex-start', gap: 10, padding: '12px 18px', borderTop: `1px solid ${T.line}` }}>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: T.ink }}>{line}</div>
        {fallback}
      </div>
      {notice.link && notice.url ? (
        <button onClick={() => void openLink(notice.url as string)} style={{ fontSize: 12, fontFamily: T.fontSans, border: `1px solid ${T.line2}`, borderRadius: 6, background: T.paper, color: T.ink, padding: '6px 10px', cursor: 'pointer' }}>
          {NOTICE_LINK_LABEL[notice.link]}
        </button>
      ) : null}
    </div>
  )
}
```

  and where `FrozenRow` renders: `{accountView?.notice ? <NoticeRow notice={accountView.notice} /> : frozenAt ? <FrozenRow frozenAt={frozenAt} /> : null}` (keep `FrozenRow` for when no document has arrived; its text holds no address). Use the file's existing `T` tokens; match the approved Windows mock for spacing.

- [ ] **Step 4: Bring the existing tests in line**

- `tests/macosAccountPlanUi.test.ts`: the check `expect(src).toContain('BILLING_URL')` becomes `expect(src).toContain('planButton(')`.
- `tests/macSettingsTabs.test.tsx`: add to the `AccountTab` bindings `useAccountView: () => ({ status: 'failed' })`, `planButton`, `linkFor`, `AccountNoticeLine: () => null`, and run the real hook with `hookModules: [{ file: 'accountScreens.tsx', name: 'useLinkOpener', bindings: { LinkFallback: () => null, openAccountLink } }]` (`openAccountLink` from `../src/accountLinks`, which calls the harness's `plugin:opener|open_url`); the open-url expectation at the plan button becomes `{ url: BUILT_IN_LINKS.billing }` (no view loaded in that test). Any other test that mounts one of the six sites gets the same `hookModules` entry.
- `tests/macSettingsModel.test.ts`: `expect(HELP_URL).toBe('https://beebeeb.io/support')` stays as it is (the value is unchanged).

- [ ] **Step 5: Run, mutation-check, commit**

```bash
cd $WT && bun test > $EVID/t18-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t18-all.log
```

Expected: `0 fail`; the pass count is `B_bun` + 61 minus Task 17's deleted spec A tests (Global Constraints); the scan test passes with no offenders. Mutations (paste each failure, revert):
1. Put `const BILLING = 'https://app.beebeeb.io/billing'` back into `src/WindowsApp.tsx`. Expected: the scan test names that file.
2. Make `planButton` ignore the notice. Expected: `a notice’s link replaces the plan button`.
3. In `src/pages/VersionCenter.tsx`, open the billing link with `openUrl(linkFor(accountView, 'billing'))` again. Expected: `each link site uses the one opener and renders its fallback` (`direct` names the call for that site).
4. In `NoticeRow`, delete `{fallback}`. Expected: the same test does not fail on it alone, because the file still renders the billing button's `{fallback}`; so also delete that one, and expect the test to fail for `windows/views/AccountView.tsx` (`fallback: false`). Revert both.

```bash
bunx tsc --noEmit > $EVID/t18-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t18-eslint.log 2>&1; echo "rc=$?"
git add src/accountScreens.tsx src/accountLinks.ts src/MacSettings.tsx src/windows/views/AccountView.tsx src/WindowsApp.tsx src/pages/Account.tsx src/pages/VersionCenter.tsx src/desktopApi.ts src/macSettingsModel.ts tests/accountNotices.test.tsx tests/accountLinks.test.ts tests/macSettingsTabs.test.tsx tests/macosAccountPlanUi.test.ts
git commit -m "desktop: show the account notice in Settings and open every billing link the server names" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/accountScreens.tsx src/accountLinks.ts src/MacSettings.tsx src/windows/views/AccountView.tsx src/WindowsApp.tsx src/pages/Account.tsx src/pages/VersionCenter.tsx src/desktopApi.ts src/macSettingsModel.ts tests/accountNotices.test.tsx tests/accountLinks.test.ts tests/macSettingsTabs.test.tsx tests/macosAccountPlanUi.test.ts
git show --stat HEAD
```

---

## Task 19: Windows — Update required and No plan in its existing windows (spec §7.7)

**Lane T.** No launch policy and no click change on Windows (§6). `windows-onboarding` shows the sign-in (Task 16), then, after sign-in and unlock and before "Files on demand", Update required or No plan when that state holds; `main-app` renders them in place of its content.

**Files:**
- Modify: `src/accountScreens.tsx` (`accountGateScreen`, `AccountGate`)
- Modify: `src/WindowsFirstRun.tsx` (`WindowsFirstRunView`), `src/WindowsApp.tsx` (`renderContent`)
- Test: `tests/windowsAccountGate.test.tsx` (new)

**Interfaces:**
- Consumes: Task 15 `useAccountView`, `AccountView`; Task 17 `AccountUpdate`, `AccountChoosePlan`.
- Produces: `accountGateScreen(view: AccountView | null): 'update' | 'choose_plan' | null`; `AccountGate({ view })` (renders `AccountUpdate` or `AccountChoosePlan`, or nothing).

- [ ] **Step 1: Write the failing tests**

```tsx
/** Spec 2026-10-07 §7.7: Windows shows Update required and No plan in its existing windows. */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { accountGateScreen } from '../src/accountScreens'
import type { AccountState, AccountView } from '../src/accountView'

const view = (state: AccountState, session: 'none' | 'valid' | 'ended' = 'valid') =>
  ({ state, session, offline: false, screen: 'nothing', busy: false, notice: null,
     links: { create_account: 'a', billing: 'b', support: 'c', step: 'd' } }) as AccountView

describe('accountGateScreen', () => {
  test('update required and no plan, and nothing else', () => {
    expect(accountGateScreen(view('update_required'))).toBe('update')
    expect(accountGateScreen(view('update_required', 'none'))).toBe('update')
    expect(accountGateScreen(view('no_plan'))).toBe('choose_plan')
    for (const state of ['checking', 'signed_out', 'session_ended', 'locked', 'ready'] as const) expect(accountGateScreen(view(state))).toBeNull()
    expect(accountGateScreen(null)).toBeNull()
  })
})

describe('the Windows windows render the gate', () => {
  const source = (file: string) => readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8')

  test('windows-onboarding: Update before the sign-in; Update or No plan before Files on demand', () => {
    const firstRun = source('WindowsFirstRun.tsx')
    const view = firstRun.slice(firstRun.indexOf('function WindowsFirstRunView('))
    expect(view).toContain("step === 'signin' && gate === 'update'")
    expect(view).toContain("step === 'sync-mode' && gate !== null")
    expect(view).toContain('<AccountGate view={accountView} />')
  })

  test('main-app: in place of its content while signed in', () => {
    const app = source('WindowsApp.tsx')
    const content = app.slice(app.indexOf('const renderContent = () => {'))
    const gate = content.indexOf('accountGateScreen(accountView) !== null')
    const routes = content.indexOf('switch (activeNav)')
    expect(gate).toBeGreaterThan(-1)
    expect(gate).toBeLessThan(routes)
  })
})
```

Run: `bun test tests/windowsAccountGate.test.tsx > $EVID/t19-red.log 2>&1; echo "rc=$?"`. Expected: `rc=1`.

- [ ] **Step 2: Implement**

`src/accountScreens.tsx`:

```tsx
/** §7.7 and C-R8: the states that take a Windows window over: Update required (whatever the session) and No plan. */
export function accountGateScreen(view: AccountView | null): 'update' | 'choose_plan' | null {
  if (view?.state === 'update_required') return 'update'
  if (view?.state === 'no_plan') return 'choose_plan'
  return null
}

export function AccountGate({ view }: { view: AccountView | null }) {
  const gate = accountGateScreen(view)
  if (!view || gate === null) return null
  return gate === 'update' ? <AccountUpdate view={view} /> : <AccountChoosePlan view={view} />
}
```

`WindowsFirstRunView`: at the top `const accountLoad = useAccountView(); const accountView = accountLoad.status === 'ready' ? accountLoad.view : null; const gate = accountGateScreen(accountView)`. In its step rendering, before the existing branches:

```tsx
        {step === 'signin' && gate === 'update' ? <AccountGate view={accountView} /> : null}
        {step === 'sync-mode' && gate !== null ? <AccountGate view={accountView} /> : null}
```

and guard the existing `step === 'signin'` and `step === 'sync-mode'` branches with `gate !== 'update'` and `gate === null` respectively, so exactly one renders. When the state becomes Ready the step renders as before (the window re-renders; nothing advances by itself).

`WindowsApp.renderContent`: after the `supportsRoute` and signed-out checks, before `switch (activeNav)`:

```tsx
    if (loggedIn && accountGateScreen(accountView) !== null) {
      return <AccountGate view={accountView} />
    }
```

(`accountView` from the `useAccountView()` Task 18 added.)

- [ ] **Step 3: Run, mutation-check, commit**

`bun test > $EVID/t19-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t19-all.log`. Expected `0 fail` and `B_bun` + 64 pass, minus Task 17's deleted spec A tests (Global Constraints). Mutation: return `null` for `no_plan` in `accountGateScreen`; expect `update required and no plan, and nothing else`; revert.

```bash
bunx tsc --noEmit > $EVID/t19-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t19-eslint.log 2>&1; echo "rc=$?"
git add src/accountScreens.tsx src/WindowsFirstRun.tsx src/WindowsApp.tsx tests/windowsAccountGate.test.tsx
git commit -m "desktop: Windows shows update required and choose a plan in its existing windows" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/accountScreens.tsx src/WindowsFirstRun.tsx src/WindowsApp.tsx tests/windowsAccountGate.test.tsx
git show --stat HEAD
```

---

## Task 20: macOS compact window — no Drive status page; it lands on the Finder location view (spec §11, C-W3)

**Lane T.** Windows and Linux unchanged.

**Files:**
- Modify: `src/compactNavigation.ts`, `src/App.tsx`
- Test: `tests/compactNavigation.test.ts`

**Interfaces:**
- Consumes: spec A's `CompactPage`, `COMPACT_NAV_ITEMS`, `compactPageFromString`, `DesktopPlatform`.
- Produces: `compactNavItems(platform: DesktopPlatform): ReadonlyArray<CompactNavItem>`, `initialCompactPage(nav: string | null, platform: DesktopPlatform): CompactPage`, `compactPageFor(page: CompactPage, platform: DesktopPlatform): CompactPage`.

- [ ] **Step 1: Write the failing tests** (append to `tests/compactNavigation.test.ts`):

```ts
import { compactNavItems, compactPageFor, initialCompactPage } from '../src/compactNavigation'

describe('spec 2026-10-07 §11: no Drive status page on macOS', () => {
  test('macOS has no Status entry; Windows and Linux keep it', () => {
    expect(compactNavItems('macos').map((item) => item.id)).not.toContain('status')
    for (const platform of ['linux', 'windows', 'unknown'] as const) expect(compactNavItems(platform).map((item) => item.id)).toContain('status')
  })

  test('the macOS compact window lands on the Finder location view, and a status link goes there too', () => {
    expect(initialCompactPage(null, 'macos')).toBe('finder')
    expect(initialCompactPage('status', 'macos')).toBe('finder')
    expect(initialCompactPage('account', 'macos')).toBe('account')
    expect(compactPageFor('status', 'macos')).toBe('finder')
  })

  test('Linux keeps landing on Status', () => {
    expect(initialCompactPage(null, 'linux')).toBe('status')
    expect(initialCompactPage('status', 'linux')).toBe('status')
    expect(compactPageFor('status', 'linux')).toBe('status')
  })
})
```

Run it; expected red (missing exports).

- [ ] **Step 2: Implement**

`src/compactNavigation.ts`:

```ts
/** Spec 2026-10-07 §11: macOS has no Drive status page (until spec B, its compact window lands on the Finder location
 *  view). Windows and Linux keep it. */
export function compactNavItems(platform: DesktopPlatform): ReadonlyArray<CompactNavItem> {
  return platform === 'macos' ? COMPACT_NAV_ITEMS.filter((item) => item.id !== 'status') : COMPACT_NAV_ITEMS
}

export function compactPageFor(page: CompactPage, platform: DesktopPlatform): CompactPage {
  return platform === 'macos' && page === 'status' ? 'finder' : page
}

export function initialCompactPage(nav: string | null, platform: DesktopPlatform): CompactPage {
  return compactPageFor(compactPageFromString(nav) ?? 'status', platform)
}
```

`src/App.tsx`: the platform tag Rust puts on the URL (`?platform=macos`, see `main.tsx`) decides before anything loads:

```tsx
const URL_PLATFORM: DesktopPlatform = new URLSearchParams(window.location.search).get('platform') === 'macos' ? 'macos' : 'unknown'

function initialPage(): Page {
  return initialCompactPage(new URLSearchParams(window.location.search).get('nav'), URL_PLATFORM)
}
```

the nav list renders `compactNavItems(URL_PLATFORM)` instead of `COMPACT_NAV_ITEMS`, keeping its `.filter((item) => supportsRoute(caps, item.id))` exactly as it is (`compactNavItems(URL_PLATFORM).filter((item) => supportsRoute(caps, item.id)).map(…)`), and every `setPage(x)` that comes from navigation (the nav buttons, `COMPACT_APP_NAV_EVENT`, `onNavigate`) goes through `setPage(compactPageFor(x, URL_PLATFORM))`. The `case 'status'` route stays for Linux.

- [ ] **Step 3: Run, mutation-check, commit**

`bun test > $EVID/t20-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t20-all.log`. Expected `0 fail` and `B_bun` + 67 pass, minus Task 17's deleted spec A tests. Mutation: make `compactNavItems` return `COMPACT_NAV_ITEMS` always; expect `macOS has no Status entry…`; revert.

```bash
bunx tsc --noEmit > $EVID/t20-tsc.log 2>&1; echo "rc=$?"; bun run lint > $EVID/t20-eslint.log 2>&1; echo "rc=$?"
git add src/compactNavigation.ts src/App.tsx tests/compactNavigation.test.ts
git commit -m "desktop: on macOS the compact window opens on the Finder location view" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- src/compactNavigation.ts src/App.tsx tests/compactNavigation.test.ts
git show --stat HEAD
```

---

## Task 21: Lane T on top of Lane R — the cross-language pins and the docs

**Lead step 1, then Lane T.** Runs after Lane R's PR has merged.

**Files:**
- Modify: `tests/accountView.test.ts`, `tests/accountLinks.test.ts`, `tests/accountViewCopy.test.ts` (pins against the Rust sources)
- Modify: `CLAUDE.md` (desktop), `docs/CAPABILITIES.md`, `docs/specs/2026-10-07-desktop-first-run-sign-in.md` (an **Amended** line only if Task 1's approval changed anything)
- Workspace (lead commits): `$WS/.claude/skills/beebeeb-designs.md`

**Interfaces:**
- Consumes: Lane R's files on `main`: `src-tauri/tests/fixtures/account-view/*.json`, `src-tauri/src/account_view/links.rs`, `src-tauri/src/tray_presentation.rs`.
- Produces: nothing new; three tests that fail if either language renames a field, an address or a tooltip.

- [ ] **Step 1 (lead): rebase Lane T**

```bash
git -C $WS/repos/desktop fetch origin
git -C ~/code/bb-worktrees/desktop-1747-t rebase origin/main > $EVID/t21-rebase.log 2>&1; echo "rc=$?"
```

Expected `rc=0`. On a conflict, resolve it in Lane T's files only (Lane R's are on `main`), record each resolution in the Notes, and run `bun test` before handing the tree back.

- [ ] **Step 2: The pins**

`tests/accountView.test.ts`:

```ts
  test('the Rust examples are exactly the ones this file parses', () => {
    const dir = new URL('../src-tauri/tests/fixtures/account-view/', import.meta.url)
    const rust = (name: string) => JSON.parse(readFileSync(new URL(name, dir), 'utf8'))
    expect(rust('signed_out.json')).toEqual(SIGNED_OUT)
    expect(rust('ready_trial.json')).toEqual(READY_TRIAL)
    expect(rust('no_plan_offline.json')).toEqual(NO_PLAN_OFFLINE)
    for (const name of ['signed_out.json', 'ready_trial.json', 'no_plan_offline.json']) expect(parseAccountView(rust(name))).not.toBeNull()
  })
```

(add `import { readFileSync } from 'node:fs'`).

`tests/accountLinks.test.ts`:

```ts
  test('the built-in addresses are Rust’s', () => {
    const rust = readFileSync(new URL('../src-tauri/src/account_view/links.rs', import.meta.url), 'utf8')
    const constant = (name: string) => rust.match(new RegExp(`pub const ${name}: &str = "([^"]+)";`))?.[1]
    expect({ create_account: constant('CREATE_ACCOUNT_URL'), billing: constant('BILLING_URL'), support: constant('SUPPORT_URL') }).toEqual({ ...BUILT_IN_LINKS })
  })
```

`tests/accountViewCopy.test.ts` (plan "Spec issues" 10):

```ts
  test('the menu-bar tooltips are the approved words (they live in Rust)', () => {
    const rust = readFileSync(new URL('../src-tauri/src/tray_presentation.rs', import.meta.url), 'utf8')
    for (const [name, words] of [['SIGNED_OUT_TOOLTIP', 'Beebeeb: Signed out'], ['SIGN_IN_AGAIN_TOOLTIP', 'Beebeeb: Sign in again'], ['OFFLINE_TOOLTIP', 'Beebeeb: Offline']]) {
      expect(rust).toContain(`pub const ${name}: &str = "${words}";`)
    }
  })
```

Run each new test, then break each pin once (change `BILLING_URL` in `links.rs` in the working tree only; rename `trial_ends` in `ready_trial.json`; change a tooltip) to see it fail; restore with an edit back, never `git checkout`. Paste the failures.

- [ ] **Step 3: Docs**

`CLAUDE.md` (desktop), a new section after "Re-sign-in in place (ruling R8)":

```markdown
### First run and the account view (spec `docs/specs/2026-10-07-desktop-first-run-sign-in.md`)

One Rust task (`src-tauri/src/account_view/`) fetches `GET /api/v1/onboarding` (anonymous when signed out, Bearer when
signed in; headers `X-Beebeeb-Onboarding-Schema: 1` and `X-Beebeeb-Client-OS`), binds every result to the session
generation and the account, caches the account part in `state.db` (`onboarding_account_cache`, purged on sign-out), and
derives one `AccountView` (`account_view_state`, event `account-view-changed`). Every window and the menu-bar icon only
render it. The account gate (`AppState.account_gate`) holds engine starts and the Finder add while the account is being
checked, needs an update or has no plan (never on Linux); it never removes Finder. macOS: the `onboarding` window is the
account window (`?platform=macos`), a login launch opens nothing (`launch_kind`), the tray shows a crossed b when
disconnected. Commands: `account_view_state`, `account_view_retry`, `account_view_plan_opened`, `quit_app`,
`show_settings_on_account`. Copy: `src/accountViewCopy.ts`; built-in addresses: `src/accountLinks.ts` only.
```

`docs/CAPABILITIES.md`, a row in the surface table:

```markdown
| Account (sign-in, plan, notices) | macOS: account window; Linux: onboarding sign-in | Windows: first-run sign-in, Update/Choose a plan in place of content, Account notice | account_view_state / account_view_retry / account_view_plan_opened / start_browser_login / quit_app |
```

If the lead's Task 1 approval changed any word or behaviour, add under the spec's header `**Amended:** <date> for the approved mocks (§12). — lead, <date>` and strike-and-replace the changed lines in place.

Workspace (lead): add `design/desktop-first-run/` (four HTML files and `tray-icon/`) to `$WS/.claude/skills/beebeeb-designs.md`.

- [ ] **Step 4: Commit**

```bash
cd $WT && bun test > $EVID/t21-all.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t21-all.log
git add tests/accountView.test.ts tests/accountLinks.test.ts tests/accountViewCopy.test.ts CLAUDE.md docs/CAPABILITIES.md
git commit -m "desktop: pin the account view's contract across Rust and TypeScript; document the first run" -m "Co-Authored-By: <your model> <noreply@anthropic.com>" -- tests/accountView.test.ts tests/accountLinks.test.ts tests/accountViewCopy.test.ts CLAUDE.md docs/CAPABILITIES.md
git show --stat HEAD
```

Expected: `0 fail` and `B_bun` + 70 pass, minus Task 17's deleted spec A tests (Global Constraints; + 2 if T11-M5 ran). Add the spec file to both lists if it was amended.

---

## Task 22: The gates (lead)

**Lead only.** Each lane's head in a fresh tree carrying only its own commits, then the merged `main`. Counts, never adjectives.

**Files:**
- Evidence only: `$EVID/t22-*.log`

**Interfaces:**
- Consumes: Task 0's baselines; both lanes' heads; the merged `main`.
- Produces: the truth lines Tasks 23 and 24 quote.

- [ ] **Step 1: Rust, in a fresh tree at Lane R's head**

```bash
git -C $WS/repos/desktop fetch origin
git -C $WS/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-1747-gate origin/feat/1747-first-run-sign-in
cd ~/code/bb-worktrees/desktop-1747-gate/src-tauri
$LOCK cargo-build -- cargo test --locked --no-run > $EVID/t22-no-run.log 2>&1; echo "rc=$?"
SCRATCH=$(mktemp -d)
RUSTUP_HOME="$HOME/.rustup" CARGO_HOME="$HOME/.cargo" HOME="$SCRATCH" $LOCK cargo-build -- cargo test --locked > $EVID/t22-cargo-test.log 2>&1; echo $? > $EVID/t22-cargo-test.exit
rm -rf "$SCRATCH"
python3 ../scripts/assert-cargo-test-counts.py $EVID/t22-cargo-test.log --cargo-exit-code $(cat $EVID/t22-cargo-test.exit)
/usr/bin/grep "test result:" $EVID/t22-cargo-test.log
/usr/bin/grep -cE "^test account_view::.* \.\.\. ok$" $EVID/t22-cargo-test.log
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/t22-clippy.log 2>&1; echo "rc=$?"
/usr/bin/grep -c '^warning' $EVID/t22-clippy.log; cat $EVID/t0-baseline-clippy-warnings.txt
/usr/bin/grep -A3 '^warning' $EVID/t22-clippy.log | /usr/bin/grep -cE "account_view/|tray_presentation.rs|launch_kind.rs"
/usr/bin/grep -c '^\[\[package\]\]' Cargo.lock; cat $EVID/t0-baseline-lock-packages.txt
git -C .. diff origin/main --stat -- bun.lock
```

Expected: `--no-run` `rc=0`; every binary `test result: ok. N passed; 0 failed`; on macOS the lib's N is exactly `B_lib` + 181 (Global Constraints, Task 14's running total; + 1 if Task 12's M5 steps ran) and every other binary's N is Task 0's; `account_view::` alone counts 117 passing tests (doc 10, links 7, notice 12, derive 22, policy 15, fetch 11, gate 6, driver 34); clippy warnings not above the baseline and 0 in the new files; the `[[package]]` count equal to Task 0's; `bun.lock` unchanged. Paste every `test result:` line into the task Notes.

- [ ] **Step 2: TypeScript, in a second fresh tree at Lane T's head (after Task 21)**

A new detached worktree, never a checkout inside the first one:

```bash
git -C $WS/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-1747-gate-t origin/feat/1747-first-run-sign-in-ui
cd ~/code/bb-worktrees/desktop-1747-gate-t && bun install --frozen-lockfile > $EVID/t22-bun-install.log 2>&1; echo "rc=$?"
bun test > $EVID/t22-bun-test.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "^ *[0-9]+ (pass|fail)" $EVID/t22-bun-test.log
bunx tsc --noEmit -p . > $EVID/t22-tsc.log 2>&1; echo "tsc rc=$?"
bun run lint > $EVID/t22-eslint.log 2>&1; echo "eslint rc=$?"
```

Expected: `N pass` with N = `B_bun` + 70 minus the spec A tests Task 17 deleted (named in its Notes; + 2 if T11-M5 ran); `0 fail`; tsc and eslint `rc=0`.

- [ ] **Step 3: After both PRs merge, on `main`**

Remove both gate trees (`git -C $WS/repos/desktop worktree remove ~/code/bb-worktrees/desktop-1747-gate` and `…-gate-t`), then repeat steps 1 and 2 with `origin/main` in place of each branch, in fresh detached trees again. Then run the workspace's contract guard, which now finds the desktop copy:

```bash
cd $WS && git -C repos/desktop pull --ff-only > $EVID/t22-pull.log 2>&1; echo "rc=$?"
bash scripts/check-onboarding-contract.sh > $EVID/t22-contract-guard.log 2>&1; echo "rc=$?"; /usr/bin/grep -E "client cop" $EVID/t22-contract-guard.log
```

Expected `rc=0` and a line counting at least 1 client copy, the desktop's among them. Remove both gate trees afterwards with `git worktree remove` as above.

---

## Task 23: Verification 1747 — macOS, on Guus's own Mac account (lead)

**Lead only.** Each row below is one rung of the spec's rewritten 1747 Verification (§15), in the same order. Nothing here touches production; every sign-in is against the local API.

**Files:**
- Evidence: `$EVID/1747-*.png`, `$EVID/1747-*-lifecycle.log`, `$EVID/1747-state-sql.md`, `$EVID/1747-notes.md`
- Modify (workspace, lead commits): `.claude/tasks/<dir>/1747-*.md` (`## Verification evidence`)

**Interfaces:**
- Consumes: the merged `main`; the local stack; spec A's Task 21 build and evidence helpers.
- Produces: the 1747 evidence, or each rung's honest open line.

- [ ] **Step 1: Two builds, two kinds of isolation**

The spec's rung "a debug `.app` bundle, `BB_API_BASE=http://localhost:3001` and `HOME=<scratch>` for every login" holds for every rung that does not need Finder. A sandboxed build's home is its container, so a scratch `HOME` cannot reach it, and the Finder rungs need the signed, sandboxed File Provider build. So:

- **Build U (unsandboxed debug bundle)** for the sign-in, notice, icon, link and launch-window rungs:

```bash
git -C $WS/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-1747-qa origin/main
QA=~/code/bb-worktrees/desktop-1747-qa; cd $QA && bun install --frozen-lockfile
$LOCK cargo-build -- bunx tauri build --debug --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked > $EVID/1747-build-u.log 2>&1; echo "rc=$?"
APPU=$QA/src-tauri/target/debug/bundle/macos/Beebeeb.app
SCRATCH=$(mktemp -d); echo "$SCRATCH" > $EVID/1747-scratch-home.txt
runu() { BB_API_BASE=http://localhost:3001 HOME="$SCRATCH" "$APPU/Contents/MacOS/$(defaults read "$APPU/Contents/Info" CFBundleExecutable)" > $EVID/1747-run-u.log 2>&1 & echo $! > $EVID/1747-run-u.pid; }
```

  (Quit it with the app's own "Quit Beebeeb", or `kill $(cat $EVID/1747-run-u.pid)`; never by name.) Its lifecycle log is `$SCRATCH/Library/Logs/Beebeeb/lifecycle.log`.

  **Precondition for every Build U rung (plan review M12):** a scratch `HOME` moves the app's config and logs, but not the macOS Keychain, which belongs to the macOS user, and spec A's `keychain_session_present` keeps a legacy read fallback. Before the first rung, `runu` with the fresh `$SCRATCH` and confirm the window shows "Sign in to Beebeeb" with the crossed b (`1747-precondition-signed-out.png`). If it shows any session, **stop**: a session from this macOS user's Keychain is in play. Sign the installed app out first, or run Build U under a separate macOS user, and record which.
- **Build S (signed, sandboxed)** for the Finder rungs and the login-launch rung: spec A's Task 21 steps 3 and 4 exactly (signing identity, both provisioning profiles, the installed app moved aside, `launchctl setenv BB_API_BASE http://localhost:3001`, `mark`/`collect`), including its precondition: Guus's installed app reports nothing waiting to upload (screenshot), and it is restored at the end.

The lead accepted this two-build split on 2026-10-07 and amends 1747's Verification line in the workspace (strike "and `HOME=<scratch>` for every login", write "`HOME=<scratch>` for every login on the unsandboxed bundle; the Finder rungs on the signed, sandboxed build in its own container, with spec A Task 21's precondition", signed and dated). Before the Finder rungs, check that the amended line is in 1747's task file.

Start the local stack with plan-less accounts landing in `needs_plan`: `cd $WS && BB_ENTRY_ALLOWANCE_BYTES=0 make dev-native` (API :3001, web :5173), then `curl -sf localhost:3001/health`. Accounts: create them in the local web app (`http://localhost:5173/signup`; signup is web-only) with `@beebeeb.io` addresses, one with 2FA turned on in its web settings. Record them (never passwords) in `$EVID/1747-notes.md`.

Account states come from the local database (the workspace's `docker compose` Postgres on :5434; credentials in the workspace's own dev setup notes). What each state needs is `repos/server/beebeeb-api/src/onboarding/derive.rs::derive_state`'s gate order: frozen (`users.frozen_at IS NOT NULL`), read-only (the newest `subscriptions` row's `billing_state = 'read_only'`), trial cancelling, then plan-less states (trial ended, lapsed, needs plan), trialing, past due (`billing_state = 'grace'` on the newest row), active. The rows `onboarding/load.rs` reads are the ones to change. Write every statement used into `$EVID/1747-state-sql.md` before running it; for example:

```sql
UPDATE users SET frozen_at = now() WHERE email = 'qa-1747-frozen@beebeeb.io';
UPDATE subscriptions SET billing_state = 'grace'
  WHERE id = (SELECT s.id FROM subscriptions s JOIN users u ON u.id = s.user_id
              WHERE u.email = 'qa-1747-pastdue@beebeeb.io' ORDER BY s.created_at DESC LIMIT 1);
INSERT INTO server_config (key, value) VALUES ('onboarding_min_client_versions', '{"desktop":{"min":"99.0.0"}}')
  ON CONFLICT (key) DO UPDATE SET value = excluded.value;   -- update required; DELETE the row afterwards
```

The no-card trial needs `server_config.no_card_trial_enabled = 'true'` locally; it starts from the local web app's billing page.

- [ ] **Step 2: Run the rungs**

| 1747 rung (spec §15) | Build | Do | Pass means (evidence) |
|---|---|---|---|
| Debug bundle, local API, scratch HOME | U, S | Step 1 | `$EVID/1747-build-u.log` `rc=0`; `ls $SCRATCH/Library` shows the app's config and logs there, none under the real home (`stat` of the real `~/Library/Application Support/beebeeb/desktop.toml` unchanged: record mtimes before and after) |
| Sign in by browser handoff | U | Launch (signed out, fresh scratch). "Sign in with browser"; approve in the local web app (`localhost:5173`, the code typed there) | The window goes busy, then Ready: no recovery-phrase step on this Mac with no key (C-S1). `1747-browser-busy.png`, `1747-browser-ready.png`; lifecycle `account_state from=checking to=ready` |
| Sign in by password | U | Fresh scratch. "Use email and password", sign in | The recovery-phrase step (no key on this Mac), then Ready. `1747-password.png` |
| Sign in by password + 2FA | U | Fresh scratch, the 2FA account | The code step, then the recovery phrase, then Ready. `1747-2fa.png` |
| Relaunch after a revoked session | U | Signed in; revoke the desktop session in the local web app (Settings → sessions); quit; relaunch | The window opens on "Sign in again" with "Sync is paused until you sign in again." (`1747-session-ended.png`); the crossed b; tooltip "Beebeeb: Sign in again" (`1747-tooltip-sign-in-again.png`) |
| Relaunch after a sign-out by choice | U | Sign out in Settings; quit; relaunch | "Sign in to Beebeeb" (Signed out), crossed b, "Beebeeb: Signed out" (`1747-signed-out.png`) |
| "Create account" opens the server's address | U | Signed out: click "Create an account on beebeeb.io" | The browser opens exactly the local document's `signup.web_url` (`curl -s localhost:3001/api/v1/onboarding -H 'X-Beebeeb-Onboarding-Schema: 1' -H 'X-Beebeeb-Client: desktop' -H 'X-Beebeeb-Client-OS: macos' \| jq -r .signup.web_url` → `$EVID/1747-signup-url.txt`); the browser's address equals it (`1747-create-account.png`) |
| A `needs_plan` account: no Finder, no engine | S | `mark NP`; sign in with a fresh plan-less account | "Choose a plan" (`1747-choose-plan.png`); `NP-lifecycle.log` has `engine_held reason=account_blocking` and no `transition … to=adding`; `"$CTL" status` = `missing`; `ls ~/Library/CloudStorage` has no Beebeeb entry. Click "Choose a plan on beebeeb.io" (it opens the built-in web-billing address until task 1844 ships: close that tab, do nothing there), then start the no-card trial in the local web app (`localhost:5173`) | "Waiting for your plan. This window updates by itself." (`1747-waiting.png`); within the 3 s poll the window continues by itself; spec A adds Finder (`collect NP`: a `to=ready` transition, `"$CTL" status` = `installed`) |
| Read-only, lapsed, frozen, payment-failed notices | U | One account per state (step 1's SQL), signed in, Settings → Account | Each line and its link (`1747-notice-read-only.png`, `…-lapsed.png`, `…-frozen.png`, `…-payment-failed.png`); the dates equal the document's `lifecycle` days (`jq .account.lifecycle` of the same account's document, saved beside each); each link opens the document's `fallback.url` when present, else the built-in address |
| Trial notice | U | The trialing account | "Free trial until {day month}." with "Manage plan" (`1747-notice-trial.png`) |
| Update-required screen | U | step 1's `server_config` row; relaunch | "Update Beebeeb" (`1747-update.png`); "Update now" shows the manual-check result as Settings does (`1747-update-now.png`). Delete the row afterwards |
| The sign-in-again window | U | from the revoked-session rung | `1747-session-ended.png` (above) |
| An icon click in each state other than Ready (C-W3) | U, S | In each of Signed out, Session ended, Update required, Locked (key in the Keychain: S; no key: U), Checking (quit, relaunch, click within the restore), No plan: click the icon; in Signed out also right-click | The window per C-W3 (one screenshot each, `1747-click-<state>.png`); Locked with the Keychain key opens Settings on its Account tab; with no session, the right click opens the window too (no menu) |
| Reopen from Spotlight (C-W2) | U | App running, no window; open "Beebeeb" from Spotlight | The window for the current state (`1747-reopen.png`) |
| A failed first Finder add after sign-in (§7.6) | S | Turn Beebeeb off in System Settings → File Providers; sign in | The account window shows "Beebeeb is turned off in System Settings." with "Open System Settings" (`1747-first-add-failed.png`) |
| "Browser won't open" | U | Make the opener fail in this window only (nothing system-wide changes): in Build U's Web Inspector (Safari → Develop → Beebeeb), run `window.__TAURI_INTERNALS__.invoke = ((orig) => (cmd, args) => cmd === 'plugin:opener\|open_url' ? Promise.reject('blocked for QA') : orig(cmd, args))(window.__TAURI_INTERNALS__.invoke)`, then click "Create an account on beebeeb.io" | The address as selectable text and "Copy link"; "Copy link" puts exactly that address on the clipboard (`pbpaste > $EVID/1747-copy-link.txt`) (`1747-link-fallback.png`) |
| The crossed b, light and dark | U | Signed out, Session ended, and offline (Wi-Fi off after two failed fetches ≥ 10 s apart): switch System Settings → Appearance between Light and Dark | Six menu-bar captures (`screencapture -R` of the status-item area), `1747-tray-<state>-<light|dark>.png`; the tooltip in each |
| Manual launch shows the centred window; a login launch shows only the icon | S | Signed out. `open -a /Applications/Beebeeb.app` | The centred "Sign in to Beebeeb" window (`1747-manual-launch.png`). The login half: only if "Start at login" actually starts this signed, sandboxed build at the next OS login (task 1845's finding). If it does, the app's log line `launch kind read from the open-application event` shows `bridge=1` (the `keyAELaunchedAsLogInItem` keyword) or `bridge=2` (`keyAEPropData` carrying `'lgit'`): record which in the Notes (spec §16), with only the icon showing (`1747-login-launch.png`). If it does not, write: "OPEN — login launch: autostart cannot start the sandboxed build (task 1845); which Apple-event field the OS sets at a login launch (spec §16, `bridge=1|2`) is open with it" |
| Wake while offline re-probes (plan "Spec issues" 21) | U | Signed in, Ready. Wi-Fi off until the Offline overlay shows (two failures ≥ 10 s apart). Sleep the Mac (Apple menu → Sleep) for at least 2 minutes, wake it, Wi-Fi on | The overlay clears by itself within 30 s of Wi-Fi being back, with no click (`1747-wake-offline.png`, `1747-wake-online.png`; the lifecycle log's `account_state … offline=no` line, its time against the wake's) |
| `cargo test` per-binary counts and `bun test` counts | — | Task 22 | Paste Task 22's truth lines |
| A clean-machine first run of a notarized, Gatekeeper-checked build | — | — | "OPEN — needs a clean Mac and a notarized build (spec §16)", unless one is available; then record it |
| Server links | — | — | "OPEN until task 1844 ships: the plan, support and payment links were the built-in addresses; rerun these rows then (spec §16)" |

- [ ] **Step 3: Clean up and record**

Build S: spec A Task 21's restore (the installed app back to `/Applications`, `launchctl unsetenv BB_API_BASE`, Guus's own sign-in). Build U: `rm -rf "$SCRATCH"` (the directory `mktemp` made, never `$HOME`). Delete the `server_config` rows step 1 added. Remove the QA tree. Write the `## Verification evidence` section of 1747's task file: one line per row (`pass — <evidence>` or the exact OPEN line), the truth lines, and every amendment made. Then `make task-verify ID=1747` only if no row is a non-OPEN failure; otherwise 1747 stays in `in-review/` with the failing rows named.

---

## Task 24: Verification 1748 — Windows, on the Legion (lead)

**Lead only.** The same rungs on real Windows hardware (not a clean machine), against a local API. No launch rungs on Windows (§6).

**Files:**
- Evidence: `$WS/.claude/tasks/_qa-evidence/1748/*`
- Modify (workspace, lead commits): `.claude/tasks/<dir>/1748-*.md`

**Interfaces:**
- Consumes: the merged `main`; the Legion (`ssh legion`); a local API the Legion can reach.
- Produces: the 1748 evidence.

- [ ] **Step 1: Build and check the signature**

On the Legion, build the Windows installer from the merged `main` with the release workflow's own steps (`docs/RELEASING.md`, "Windows Authenticode signing"), through Windows' PowerShell from the WSL session (`/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe`), in a build directory under the Windows user's profile, capped below 16 build jobs (the Legion is Guus's own machine). Then run `scripts/windows/verify-authenticode.ps1` against the produced `setup.exe` and MSI, and save its output to `$EVID1748/verify-authenticode.txt`.
Expected: the script's own pass line with its file count. If the Legion cannot sign (no Azure Artifact Signing identity there), stop and amend the line in place: the signature rung is answered by the signed installer that `release.yml` builds for the same commit on an alpha channel, installed on the Legion; strike, replace, sign and date. A VM or CI run never answers a hardware rung.

The local API: run `BB_ENTRY_ALLOWANCE_BYTES=0 make dev-native` on the Mac and point the Windows app at it (`BB_API_BASE=http://<the Mac's LAN address>:3001`, set for the app's process as the Windows build documents it); `curl` its `/health` from the Legion first. Isolation: a new Windows user profile for QA, or the Windows app's state directory moved aside and restored at the end; record which.

- [ ] **Step 2: Run the rungs**

| 1748 rung (spec §15) | Do | Pass means (evidence) |
|---|---|---|
| Sign in by browser | `windows-onboarding`: "Sign in with browser"; approve in the local web app | No recovery phrase; then "Files on demand". `1748-browser.png` |
| Sign in by password | "Use email and password" | The recovery phrase, then "Files on demand". `1748-password.png` |
| Sign in by password + 2FA | the 2FA account | The code step, then on. `1748-2fa.png` |
| "Create account" opens the server's address | Signed out: the footer link | The browser address equals the local document's `signup.web_url` (fetched with `X-Beebeeb-Client-OS: windows`). `1748-create-account.png` |
| A `needs_plan` account shows "Choose a plan", sync not started | Sign in with a plan-less account | "Choose a plan" before "Files on demand" (`1748-choose-plan.png`); `main-app` shows it in place of its content (`1748-main-app-choose-plan.png`); no engine: the `tracing` log has `engine_held reason=account_blocking` and no sync activity. Start the trial in the local web app: the window continues by itself |
| The notices, a real link in the frozen one | One account per state (Task 23's SQL), Account view | Each line (`1748-notice-<state>.png`); the frozen row is the notice "This account is frozen." with "Contact support", which opens the support address (`1748-frozen-link.png`) |
| The crossed b `.ico` | Signed out; after a revoked session; offline (network off) | The colour crossed icon in the notification area, light and dark taskbar (`1748-tray-<state>-<theme>.png`), with its tooltip |
| `cargo test` counts | Run `cargo test --locked` in `src-tauri` on the Legion's Windows toolchain | Every binary `test result: ok. N passed; 0 failed`; the Windows-only binaries (`windows_session_wiring`, `windows_signout_cleanup`) among them |
| First run on a clean physical Windows machine | — | "OPEN — needs a clean physical Windows machine; a VM or CI run never answers it (spec §15)" |
| Struck rungs | — | Native signup, phrase window, ticket expiry, and acceptance item C6 (fixed by 1734, `7728a27`): listed as struck, with the reason |

- [ ] **Step 3: Record**

Restore the Windows state you moved aside. Write 1748's `## Verification evidence`: one line per row, the truth lines, the amendments. `make task-verify ID=1748` only when no row is a non-OPEN failure.

---

## Final-review notes

For the whole-branch reviewer and the lead, after Task 24.

- **Lane T rebase carry-over (spec A rulings).** Both are on surfaces this plan touches, so both are in the plan: T11-M5 (the switch warning's count is `null`, never `0`, on an unreadable `state.db`) is Task 12 steps 6–8 and Task 16 step 8, run only if Task 0 found it still open on `main`; T11f1-c2 (the retryable `SIGN_IN_ACCOUNT_UNKNOWN` sentence renders) holds on both paths: the password path already shows `result.reason`, and Task 16's `browserResult` passes the browser path's `Err` sentence through unchanged (`an error is the sentence Rust wrote, the account-unknown one included`). If Task 0 found M5 already fixed by spec A's rebase, confirm its frontend half renders `null` with words the lead approved, and note it here.
- **Spec A items marked "final review must triage"** that this plan does not change: `[T11-sec-reinstall]` (a same-account reinstall of the stored key without a server check when user ids match) and `[T11-M8]` (revoking the replaced, still-live token on a same-account swap). Spec C's browser path reaches the same `settle_sign_in`, so a ruling on either applies to both sign-in paths.
- **Spec issues found 1–21** above: each names its task and test. Issues 3 (the gate follows the account), 5 (one failure ends Checking) and 12 (Settings on the Account tab) change behaviour relative to the spec's literal wording; the lead confirmed all three on 2026-10-07. Issues 19 (the 15 s Checking limit) and 20 (the gate closed for an underived generation) are the lead's rulings on the plan review. Issue 21 (no separate wake path) is the writer's reading of plan review I8 and is open for the lead.
- **Plan review (2026-10-07, `specC-plan-review.md`).** Applied: C1 (every view, the sign-out and switch clear included, goes through one publish path: `Published` in Task 10, `AppPublisher` in Tasks 11 and 14); I1 (Task 9's generation gate); I2 (Task 17's `account-window.json`); I3 and I4 (Task 0's `t0-record.md` and signature table; the environment, the loop commands and the lane rules in Global Constraints; `bun install` in both lanes' trees); I5 (Task 6's limit, Task 10's watchdog, Task 11's drop guard); I6 (a Rust derive test, two Linux step-flow tests, the macOS test reworded with a real mutation); I7 (`useLinkOpener` at all eight link sites, pinned in Task 18); I8 (three driver tests and the busy-screen test; the wake path removed, Spec issue 21); I9 (exact counts and per-test filters, the table in Global Constraints); M1–M14. M7's second half is done as reads only where the table uses them, not as a per-generation cache: a cached owner record could outlive the sign-out's purge if the view refreshed between the generation bump and the purge.
- **The device rungs' isolation** (Task 23 step 1): the two-build split was accepted by the lead on 2026-10-07, who amends 1747's Verification line in the workspace.
- **Not in this plan:** the server's account-stage `fallback` links and the `needs_plan.desktop` fixture (workspace task 1844), and macOS autostart in the sandboxed build (workspace task 1845).

---

## Self-review (writer, 2026-10-07)

**1. Spec coverage.** Every requirement maps to a task:

| Spec | Task |
|---|---|
| §3 C1 (everything on the web), C5 (no Finder, no sync) | 5 (NoPlan), 9 (gate), 17 (one button), 19 (Windows) |
| §3 C2 (browser first) | 16 |
| §3 C3, §6.1 C-W1, C-W2 | 13 |
| §3 C4, C4a, C4b, §8 C-I1–C-I6 | 1 (assets), 14 |
| §3 C6-links, §5.1 | 3, 15, 18 |
| §3 C7 (payment failed → billing) | 4, 18 |
| §4.1 inputs, §4.2 C-R1–C-R9, C-R11, C-R13, C-R15, §4.3 | 5, 10, 11 |
| C-R10 (the gate at the point of action) | 9, 10, 11 |
| C-R12 (every edge into Ready) | 10, 11 |
| C-R14, §7.8 Linux | 5 (gate), 13/14 (Linux rows unchanged, colour icon), 16 (shared sign-in), 17 (Linux keeps its steps) |
| §5 C-D1, C-D2 | 8 |
| C-D3 | 2 |
| C-D4 | 6, 10 |
| C-D5 (generation, cache, identity, traces, purge) | 7, 10, 11 |
| C-D6 (never logged) | 8 (`Debug`), 10 (closed vocabulary) |
| §6.2 C-W3–C-W9, §6.3 C-W10–C-W11, §6.4 | 13, 14, 17 |
| §7.1 (incl. C-S1, not_in_applications notice), §7.2 | 12, 16, 17 |
| §7.3, §7.4 | 17 |
| §7.5 | 4, 15, 18 |
| §7.6 | 17 |
| §7.7 | 16, 18, 19 |
| §9 (1844) | out of scope; built-in addresses (3) |
| §10 errors table and lifecycle lines | 6, 8, 10 (the wake re-probe is the 30 s offline re-probe: Spec issue 21, checked in 23) |
| §10 "Browser won't open" | 16 (`useLinkOpener`), 17, 18 (all eight link sites) |
| §11 removals | 17, 18, 20 |
| §12 design before code | 1 |
| §13 units | 2–14 (doc split into `doc`, `links`, `notice`) |
| §14 tests (every named test) | 2–21 |
| §14 gates | 22 |
| §15 1747, 1748 | 23, 24 |
| §16 open rungs | 23, 24 |

**2. Placeholder scan.** The plan contains no "TBD", "TODO" or "implement later". `todo!()` appears only in step-1 skeletons that the next step replaces. Every code step shows the code. Task 23's SQL is given for the states whose columns the server code names (frozen, past due, update required); the others are produced from the same `load.rs` reads and written into `1747-state-sql.md` before they run, because the lead runs them against the local database's live schema.

**3. Type consistency.** These names were checked across tasks: `Derived { view, gate_open, blocking, condition }` (5 → 10, 11), `AccountGate::{arm, set(open, generation) → Option<GateValue>}` (9 → 10, 11), `GateValue::{open_for, allows_add}` (9 → 10), `policy::checking_deadline` (6 → 10), `Publisher` / `AppPublisher` (10 → 11, 14), `RestoreFinishedGuard` (11), `useLinkOpener` (16 → 17, 18), `Trigger::{AccountHold, AccountReady}` (9 → 11), `EngineStart::AccountBlocked` (9), `Facts` (10 → 11), `AccountViewHandle::{send, state, clear_for_sign_out, settled}` (10 → 11, 13, 14), `Event::{Launch, RestoreFinished, SessionChanged, WindowFocused, PlanOpened, Retry}` (10 → 11), `StartupSurface::{Onboarding, MainWindow, AccountWindow, SettingsAccount, Nothing}` (13 → 14), `open_surface`, `show_macos_settings_window_on_account` (13 → 14), `LoginOutcome::browser_signed_in`, `browser_settlement` (12), `SignInSettled.fresh.vaultUnlocked` (16 → 17), `AccountView` JSON (5 ↔ 15, pinned in 21), `BUILT_IN_LINKS` ↔ `CREATE_ACCOUNT_URL`/`BILLING_URL`/`SUPPORT_URL` (3 ↔ 15, pinned in 21), the tooltips (14 ↔ 21). `session_generation()` is the one name not yet on any branch (Spec issue 15; Task 0 records it in `t0-record.md`, with its bump sites and order).

**5. After the plan review (2026-10-07).** Recounted every task's tests against its code (the table in Global Constraints); rechecked that every test this revision adds or changes names a mutation that fails it, with the assertion that fails (tests the first version already mutation-checked keep their checks; the driver's older tests are covered by Task 10's mutations 1–7 as before); rechecked that no step writes the watch channel except through `Published::publish`, and that every `#[cfg]` added on an `if let` sits on a block.

**4. Review Focus.** Five conditions, each with its test in its owning task (Tasks 5, 10, 11, 13). Checked and not added: a sign-out during a fetch (in the spec's list: C-D5's generation test), the plan poll while offline (in the spec's list), an off-domain server link (in the spec's list), a 429 storm at launch (in the spec's list).
