# macOS Finder Write Versions Implementation Plan (task 1873, round 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On a Mac, a file saved in the Finder location reaches the server once, with the right base, whatever the timing; its bytes stay on disk and the item stays writable; our own upload never makes the system evict or re-download what it already holds; a real remote change still reaches the disk; and nothing a person saved is lost, reverted or overwritten silently.

**Architecture:** Every content write the daemon accepts from the File Provider gets a fresh write token `{b}:w{W}`. The reply names the accepted bytes with it, and one predicate keeps reporting it while the write is queued or while the server is at the version that write produced (spec §5). The system's next base is mapped through the token (§6.1): a queued token makes the save that write's successor (`after_write_id`), and a landed one gives the version it produced. Every decision that reads or changes the queue or the held columns is ONE `StateDb` method in one `BEGIN IMMEDIATE` transaction, and the runner claims each op in one transaction (§8.7). Around that core: `base_pending` waits for a snapshot to learn a missing version (§6.3), a 30-day alias resolves provisional ids after the id swap (§7), stale bases and missing payloads park at once with a lazy hand-over to the successor (§8.4), and the landing is one transaction after a recorded completion (§8.6).

**Tech Stack:** Rust 2024 (Tauri v2 daemon, rusqlite 0.32 bundled SQLite, tokio, reqwest, serde, uuid 1, tracing; no new crates), Swift (FileProvider.framework extension; the `swiftc` harness in `BeebeebFileProviderTests/main.swift`), Python guard scripts already in `scripts/`.

**Spec:** `docs/specs/2026-10-09-macos-finder-write-versions.md`, revision 4c (commit `9eced4a`). Executors read the spec and this plan together. Where they disagree, see "Spec issues found" below; nothing there is silently decided. Section and rule numbers (§5.3, rule 1a, T39, D8b) are the spec's. `EB`, `IPC`, `SD`, `RUN`, `FPE`, `FPI`, `XPC`, `ITEM.h`, `REPL.h`, `THUMB.h`, `UP`, `FILES`, `VER` and `QA/` mean what the spec's "Citations" block says. Desktop line numbers are at `e7e3dff`, whose code is identical to `931b885` (`git diff --stat 931b885 e7e3dff` touches only the spec).

---

## Spec issues found

Each item has file:line evidence. None blocks a task; each is resolved in this plan as stated, and the reviewer can reject any one on its own.

1. **A create hand-over is not "kind `upload_file`".** Spec §8.4 says that when a parked W is a create, N "takes W's create instead: kind `upload_file`". Finder creates are queued as `OperationKind::UploadVersion` whose metadata carries `"operation": "create_file"` (`EB:1604-1624`); creates are recognised by that field (`is_create_file_operation`, `EB:4058-4060`). **Plan:** N takes W's `kind` as stored and sets its own metadata's `operation` to `create_file` (Task 5).
2. **Nothing records "minted by this spec" or "from an earlier build".** §10.2, §10.3 and D1 say earlier-build ops are "marked as from an earlier build", and §6.1 rule 3 and §8.4 test "minted by this spec". §5.2 lists no column for either, and the `minted_writes` table (now split off) was not that marker. **Plan:** `operation_queue.write_origin TEXT`, `'minted'` or `'earlier_build'`, set in the accept transaction and at engine start (Tasks 1, 3, 10).
3. **§6.3.3's "10 successful snapshots that did not report the file" has no counter.** **Plan:** `base_pending` holds it: `0` = not pending; `n ≥ 1` = pending after `n − 1` such snapshots; the op parks when the count would pass 10 (Task 6).
4. **"No contract" never holds for a row that exists.** §5.4 row 1, §6.1 rule 2 and §7.4 treat a provisional row as having no contract. Every `files` row carries the contract columns, and `get_file_contract_state` returns `Some` for any existing row (`SD:2303-2314`); the `"0"` a provisional row reports comes from `item_content_version(0, None)` through the contract branch (`IPC:2578-2579`, `IPC:2642-2643`). **Plan:** "provisional" means `current_version = 0` and a create of this file is queued, in any state (Task 3).
5. **Where the restore reads "a write is queued".** §8.7 S1.6 puts that read at the restore's *enqueue*; §5.4 row 10 applies it when the restore's *response* arrives. **Plan:** the read is in the transaction that applies the response (Task 4). The enqueue needs no read: the claim orders restores (S3 step 2, Task 2).
6. **§8.7 S6 left "Unverified that `staged_payloads` fits".** Checked: `completed = 1` already means "safe to delete". The landing sets it (`EB:831`) before the unlink (`EB:1028-1030`), and the Windows sign-out reads it as "the server holds these bytes" (`staged_payload.rs:188-195`). A payload retired by a hand-over is also held elsewhere, in the successor's bytes (§8.2, last point). **Plan:** reuse it (Task 2).
7. **The Global Constraints the brief names are only half in the spec.** The spec states the identifier format (§5.1) and the server boundary (header; §12, S1). It has no line on new dependencies or public wording. **Plan:** those two are carried from the workspace rules and marked as such below.
8. **§11 lists `base_unknown` as a reason of `Finder write refused`.** Every `base_unknown` case is a park, not a refusal: rule 6a′ "queued and parked at once" (§6.1), the `init` guard "parked as `base_unknown`" (§6.3.1), and the 10-snapshot limit (§6.3.3). **Plan:** those log the `parked` line with `reason=base_unknown`; `Finder write refused` gains only `unknown_item` (Tasks 3, 8, 11).
9. **§6.3.2 asks a re-snapshot for every `file_create` op on an existing row.** Our own create's echo is such an op, and it carries `version_number` (`UP:1576`), so a snapshot would buy nothing and cost one per Finder create. **Plan:** a content op without `version_number` requests a re-snapshot, whether it inserts a row or hits an existing one. That covers the legacy chunked complete (`FILES:3656-3667`), which never carries a version (Task 6).
10. **T37 cannot cover the extension's options line, and §13 fixes the Swift count at 96** (T20, T44). The `NSLog` line is Swift. **Plan:** it is checked on the device (D9) only, and no Swift test is added for it (Task 11).
11. **Keep Mine runs `upload_version` inline, outside the queue** (`EB:3172-3200`, `max_attempts: 1`). The guarded resume write of §8.7 S4 (`INSERT … WHERE EXISTS (the op)`) would match no row for it. **Plan:** the upload path takes an optional claim; with none (Keep Mine) it keeps today's unguarded writes (Task 2).
12. **Minting must stay off the shared `queue_finder_*` entry points** (§8.8): the upload driver (`EB:2050`) and the watcher (`watcher.rs:727`, `:865`) call them too. **Plan:** two new entry points, `queue_file_provider_create_from` and `queue_file_provider_modify_from`, used by the IPC arms (`IPC:1832`, `IPC:1877`) and by the Finder tests. `queue_finder_*` keep today's behaviour (Task 3).

13. **The IPC socket is not macOS-only.** `serve_ipc` runs on every unix build (`RUN:1115-1128`), and the spec's scope is "the macOS File Provider path only … Linux behaviour is unchanged". The Windows engine also runs engine start. **Plan:** the IPC arms reach the minting entry points only on macOS (`#[cfg(target_os = "macos")]`, Task 3), and the earlier-build marking runs at engine start only on macOS (Task 10; on Windows it would hand write ids to Cloud Files uploads, whose `None` bases the `init` guard would then park). The snapshot fill and raise (Task 6) stay cross-platform: with no held token they only make a row's version more accurate. The lead may rule otherwise.
14. **D1 expects two log lines where the logger writes one.** D1's pass reads "one 409 line with `class=stale_base`, then one `parked …` line". The existing logger writes one line per attempt, the parked form of `upload refused …` (`EB:4004-4027`), and §11 caps it at "at most one line per write or per attempt". **Plan:** D1 accepts one line that carries `409`, `class=stale_base`, `parked` and the op id (Task 13).

## Global Constraints

Copied verbatim from the spec where the spec states them; the two the spec does not state are marked.

- **Identifier format** (spec §5.1, verbatim): "`{b}:w{W}`, for example `2:w9f1c0e3a5b7d4f2e8a6c4b2d0e9f7a1c`." "**`W`** is 32 lowercase hex digits: a fresh random (v4) UUID without hyphens. One is minted per accepted content write. It is never reused and never derived from a name or path." "**Size.** At most 10 + 2 + 32 = 44 bytes. Apple limits a version component to 128 bytes (`ITEM.h:85`)." "**Matching** is by exact string equality with the row's held token, never by pattern."
- **No change to the server** (spec header and §12, verbatim): "API server (private repository) `46e5054c`, read-only." "**Optional server line item S1, separate from this spec.** … Nothing here depends on it."
- **Scope** (spec header and §6.3.1, verbatim): "**Repo:** desktop. The macOS File Provider path only. Windows (Cloud Files) and Linux behaviour is unchanged." "Uploads without a `write_id` (Windows, the watcher) keep their current `None` bases."
- **Logs** (spec §11, verbatim): "Every line is a `warn!`, except the hydrate count (`info!`), with ids and fixed categories only. No name, path, URL or error text appears (the round-3 style, `EB:4004-4027`, `IPC:1406`, `IPC:2194-2203`). There is at most one line per write or per attempt, and a parked op stops logging."
- **Serialization** (spec §8.7 S1, verbatim): "Each decision below is ONE `StateDb` method. The method: takes the mutex once; opens `BEGIN IMMEDIATE` (rusqlite `TransactionBehavior::Immediate`, so the rule still holds if a second connection ever appears); reads its preconditions inside; decides with a pure function; writes, and commits. Nothing inside awaits or touches the network. Its preconditions are never read in an earlier call."
- **No new dependency** (not stated in the spec; workspace rule): `src-tauri/Cargo.lock` and `bun.lock` are unchanged at the end, and no Swift package is added. `git diff 9eced4a -- src-tauri/Cargo.lock bun.lock` is empty. The D8b helper (Task 13) lives in a scratch directory and is never committed.
- **Public wording** (not stated in the spec; workspace rule): this repo is public. Commit messages, code comments and docs use neutral wording; no private task background, no provider name, no account or person names. QA evidence is referred to as `QA/…`, as the spec does.
- **Evidence:** every new test is seen RED before it passes, and its mutation (the spec's "Mutation that must turn it red" column, or the one named here) is seen RED on the intended assertion. Logs go to `$EVID` with the `r4-` prefix (spec §13). Counts, never adjectives: `test result: ok. N passed; 0 failed; 4 ignored`.
- **Process:** one worktree for the lane; `CARGO_TARGET_DIR` unset (the tree's own `src-tauri/target`); heavy cargo runs through `$LOCK cargo-build --` (rc 75 = busy, rerun); foreground commands only; no `git stash`, `reset`, `checkout --`, `restore`, `clean`, `switch`; no `pkill`; explicit pathspecs on every commit; each commit ends with the trailer of the model that wrote it (for example `Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>`).

## Review Focus

The five input classes most likely to bite a person that no T-test in the spec exercises. Each line names the test that now pins it and the task that owns it.

1. **An autosaving editor: twenty saves in quick succession while the first upload is slow.** A person expects every save to land in order, the last one to be the file's content, and the item to stay writable throughout. Pinned by `twenty_rapid_saves_land_in_order_and_the_last_wins` (Task 4).
2. **Quitting Beebeeb, or restarting the Mac, in the middle of a save's upload.** A person expects the save to finish after the relaunch, the item to stay writable, and no re-download of the bytes on disk. Pinned by `a_restart_mid_upload_resumes_and_keeps_the_token` (Task 9).
3. **An empty file (0 bytes) saved, then read while its upload is queued.** A person expects an empty file, not an error. Pinned by `an_empty_save_lands_and_is_served_from_the_queue` (Task 8).
4. **A reply lost to an IPC timeout, followed by the next save** (the case rule 1c covered before it was split off). A person expects nothing lost; the shipped behaviour is a visible park with the bytes kept. Pinned by `a_lost_reply_base_parks_with_its_bytes` (Task 5), which the 1879 follow-up later turns into a landing.
5. **A file trashed on the web while a Finder save of it is still queued.** A person expects the save not to vanish, and the trashed item not to look writable. Pinned by `a_save_to_a_file_trashed_elsewhere_keeps_its_bytes` (Task 9).

---

## File map

Paths are relative to the desktop repo root.

| File | Status | Responsibility | Tasks |
|---|---|---|---|
| `src-tauri/src/write_token.rs` | create | the token (`WriteToken`, `mint_write_id`, `parse_token`), `HeldWrite`, the predicate `held_token`, the base mapping `decide_base` (pure) | 1, 3 |
| `src-tauri/src/lib.rs` | modify | `mod write_token;` next to `mod state_db;` (`lib.rs:68`) | 1 |
| `src-tauri/src/state_db.rs` | modify | migration; `item_presentation`; `claim_operation` and the guarded writes; `accept_finder_write`; held-column and chain writes; `apply_landing`; restore, purge, snapshot fill/raise, resnapshot counter; aliases; queue fetch; `engine_start_repair` | 1–10 |
| `src-tauri/src/engine_bridge.rs` | modify | the runner loop on claims; seams; the File Provider entry points; the `init` guard and 409 classes; parks and the hand-over; the landing; restores; snapshot hooks; alias resolution; queue-served hydrates; thumbnails; logs | 2–11 |
| `src-tauri/src/api_client.rs` | modify | `init_upload` keeps a 409's message (`InitConflict`) | 5 |
| `src-tauri/src/ipc_socket.rs` | modify | the payload builder (predicate, status), the new `QueueFinderCreate` fields, write replies, item lookup and thumbnail through the alias, `Finder hydrate served`, M5 | 4, 8, 9, 11 |
| `src-tauri/src/ipc_socket_framing_tests.rs` | modify | the reply's content version is the token | 4 |
| `src-tauri/src/runner.rs` | modify | `engine_start_repair` at engine start (`RUN:997`); the daily alias sweep | 2, 8 |
| `BeebeebFileProvider/IPCFraming.swift` | modify | `IPCWriteRequest.create` carries the three deletion-conflicted fields | 8 |
| `BeebeebFileProvider/XPCBridge.swift` | modify | `queueCreateItem` passes them | 8 |
| `BeebeebFileProvider/FileProviderExtension.swift` | modify | `createItem` reads `options`; a queued write without an item is an error; the options log line | 8, 11 |
| `BeebeebFileProviderTests/main.swift` | modify | T20, T44; the existing no-item check moves into T20 | 8 |
| `scripts/test-ipc-framing.sh` | modify | `EXPECTED_TESTS=96` | 8 |
| `docs/IPC_PROTOCOL.md` | modify | "Versions and queued writes" rewritten; skew rows; tests table | 12 |
| scratch `d8b-legacy-replace/` | scratch only | the D8b helper, never committed | 13 |

## Lane, branch and conventions

- **One lane, Tasks 1–12, strictly sequential**: each task consumes the previous one's types. Task 13 is lead-only, on the lead's Mac.
- **Tree:** the lead allocates it from the head that carries this plan. The plan writes `$WT` for its root. Suggested: `git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add ~/code/bb-worktrees/desktop-1873-r4 -b fix/1873-r4-write-versions <plan commit>`.
- **Variables used in every task:**

```bash
WT=~/code/bb-worktrees/desktop-1873-r4          # the lane's tree
LOCK=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/scripts/coord/with-lock.sh
EVID=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/tasks/_qa-evidence/1873
```

- **The test loop** (from `$WT/src-tauri`):

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib <filter> > $EVID/r4-t<N>-<step>.log 2>&1; echo "rc=$?"; grep -E "test result:|error\[|FAILED|panicked" $EVID/r4-t<N>-<step>.log | head -20
```

  If `rc=75`, the lock was busy: run it again.
- **The task gate** (end of every Rust task; `<L>` is the expected lib count the task names):

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo check --locked --all-targets > $EVID/r4-t<N>-check.log 2>&1; echo "check rc=$?"
$LOCK cargo-build -- cargo test --locked > $EVID/r4-t<N>-cargo-test.log 2>&1; echo $? > $EVID/r4-t<N>-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py $EVID/r4-t<N>-cargo-test.log --cargo-exit-code "$(cat $EVID/r4-t<N>-cargo-test.exit)"
grep "test result:" $EVID/r4-t<N>-cargo-test.log
```

  Expected: `check rc=0`; the count guard passes; the first `test result:` line (the lib) reads `ok. <L> passed; 0 failed; 4 ignored`; `keychain` 20, `windows_session_wiring` 8, `windows_signout_cleanup` 3 (the round-3 gate, `QA/lead-gate-r3-cargo-test.log`).
- **Unix-only tests.** `ipc_socket` is compiled only on unix (`lib.rs:40-41`). Every new test that names `crate::ipc_socket` (directly, or through `arm_builder_seam`, `write_outcome_response`, `file_status_response`, `file_entry_payload_for_db`, `hydrate_failure_reply` or `CAP_WRITE`) carries `#[cfg(unix)]`, as `held_content_version` already splits (`EB:12898-12911`). Windows CI then compiles without them.
- **Lib counts.** `L0` is the lib count measured in Task 1 step 0 (959 at the round-3 gate, `QA/lead-gate-r3-cargo-test.log`; re-measured, not assumed). Each task names its expected count as `L0 + k`, where `k` counts only new test functions. A test changed in place keeps the count. All counts are on macOS; `#[cfg(unix)]` IPC tests do not run on Windows CI.
- **Mutation step (every new test).** Apply the named mutation, run the test's filter, confirm RED on the named assertion, revert, run it green. Paste the RED line and the green line into the task file's Notes, with the log paths.
- **Commits:** one commit per task, explicit pathspec, the lane's own trailer. Lanes never commit `graphify-out/` and never push; the lead pushes.

---

## Task 1: Schema, the token module, the one-read presentation (spec §5.1–§5.3, §5.2; commit 1)

No behaviour changes: the columns are added and nothing writes them yet.

**Files:**
- Create: `src-tauri/src/write_token.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod write_token;` directly after `mod state_db;`, `lib.rs:68`)
- Modify: `src-tauri/src/state_db.rs`: `StateDb::open` (`SD:759-970`); `get_file_contract_state` (`SD:2303`) gains a connection-level twin; new `WriteOrigin`, `FinderWrite`, `ItemPresentation`, `item_presentation`, `finder_write`
- Test: `write_token.rs` (`mod tests`), `state_db.rs` (`mod tests`, next to `reconcile_stale_in_flight_on_startup_resets_transient_rows`, `SD:5352`)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `crate::write_token::{WRITE_ID_HEX_LEN: usize = 32, WriteToken { base: i64, write_id: String }, WriteToken::render(&self) -> String, mint_write_id() -> String, parse_token(&str) -> Option<WriteToken>, HeldWrite { write_id: String, base: i64, version: Option<i64>, object_version_id: Option<String> }, HeldWrite::token(&self) -> String, held_token(held: Option<&HeldWrite>, held_write_queued: bool, current_version: i64, current_object_version_id: Option<&str>) -> Option<String>}`
  - `crate::state_db::WriteOrigin { Minted, EarlierBuild }` with `as_str() -> &'static str` (`"minted"`, `"earlier_build"`) and `from_db(Option<&str>) -> Option<WriteOrigin>`
  - `crate::state_db::FinderWrite { write_id: String, origin: WriteOrigin, after_write_id: Option<String>, base_pending: i64 }`
  - `crate::state_db::ItemPresentation { contract: FileContractState, held: Option<HeldWrite>, held_write_queued: bool, unparked_finder_upload: bool, version_filled: bool }`
  - `StateDb::item_presentation(&self, file_id: &str) -> Result<Option<ItemPresentation>>` (one locked read)
  - `StateDb::finder_write(&self, op_id: &str) -> Result<Option<FinderWrite>>`
  - free fn `get_file_contract_state_conn(conn: &Connection, file_id: &str) -> Result<Option<FileContractState>>` (the body of `SD:2303-2335`, which then calls it)
  - SQL facts later tasks rely on: "parked" is `attempts >= max_attempts`; "a Finder upload" is an op with `write_id IS NOT NULL AND kind IN ('upload_version','upload_file')`.

- [ ] **Step 0: Record the baselines (this task only)**

```bash
cd $WT/src-tauri && git -C $WT log --oneline -1
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/r4-baseline-clippy.log 2>&1; echo "rc=$?"
grep -c '^warning' $EVID/r4-baseline-clippy.log | tee $EVID/r4-baseline-clippy-warnings.txt
$LOCK cargo-build -- cargo test --locked > $EVID/r4-baseline-cargo-test.log 2>&1; echo $? > $EVID/r4-baseline-cargo-test.exit
grep "test result:" $EVID/r4-baseline-cargo-test.log
cd $WT && bash scripts/test-ipc-framing.sh > $EVID/r4-baseline-swift.log 2>&1; echo "swift rc=$?"; tail -2 $EVID/r4-baseline-swift.log
```

Expected: clippy warnings 146 (spec §13); lib `ok. 959 passed; 0 failed; 4 ignored` (`L0`; if it differs, write the measured value into Notes and use it as `L0` everywhere below); Swift `ipc-framing: 94 passed, 0 failed`. Paste every count into the task file's Notes under "Baseline (<sha>)".

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/write_token.rs` with only the tests and empty items they name, so the tests compile and fail:

```rust
//! The write token (spec `docs/specs/2026-10-09-macos-finder-write-versions.md` §5):
//! the content version a reply names a File Provider write's bytes with.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_format_and_older_parse() {
        let w = mint_write_id();
        assert_eq!(w.len(), WRITE_ID_HEX_LEN, "{w}");
        assert!(w.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')), "{w}");
        assert_ne!(w, mint_write_id(), "a write id is never reused");

        let token = WriteToken { base: 2, write_id: w.clone() }.render();
        assert_eq!(token, format!("2:w{w}"));
        let widest = WriteToken { base: i64::from(i32::MAX), write_id: w.clone() }.render();
        assert!(widest.len() <= 44 && widest.len() <= 128, "{widest}");
        assert_eq!(parse_token(&token), Some(WriteToken { base: 2, write_id: w.clone() }));

        // Older code reads the first segment as the base (EB:4718-4726).
        assert_eq!(crate::engine_bridge::parse_base_version_number(Some(&token)), Some(2));
        assert_eq!(crate::engine_bridge::parse_base_version_number(Some(&format!("0:w{w}"))), None);

        // Never confused with the other identifier forms.
        for other in [
            "2".to_string(),
            "2:abc123".to_string(),
            "2:1700000000:10".to_string(),
            format!("2:W{w}"),
            format!("2:w{}", w.to_uppercase()),
            format!("2:w{w}0"),
            format!("-2:w{w}"),
            format!(":w{w}"),
        ] {
            assert_eq!(parse_token(&other), None, "{other}");
        }
    }

    #[test]
    fn held_token_is_some_exactly_in_the_predicate_cases() {
        let queued = HeldWrite { write_id: "a".repeat(32), base: 1, version: None, object_version_id: None };
        assert_eq!(held_token(Some(&queued), true, 1, Some("o1")), Some(queued.token()), "queued, any state");
        assert_eq!(held_token(Some(&queued), false, 1, Some("o1")), None, "neither queued nor landed");

        let landed = HeldWrite { version: Some(2), object_version_id: Some("o2".into()), ..queued.clone() };
        assert_eq!(held_token(Some(&landed), false, 2, Some("o2")), Some(landed.token()), "our own landing");
        assert_eq!(held_token(Some(&landed), false, 3, Some("o3")), None, "a remote change");
        assert_eq!(held_token(Some(&landed), false, 2, Some("o9")), None, "same number, other object");
        assert_eq!(held_token(Some(&landed), false, 2, None), None, "m-3: the row's NULL never matches");
        let no_object = HeldWrite { object_version_id: None, ..landed.clone() };
        assert_eq!(held_token(Some(&no_object), false, 2, Some("o2")), None, "m-3: the held NULL never matches");
        assert_eq!(held_token(None, true, 2, Some("o2")), None, "no held write");
    }
}
```

In `state_db.rs` `mod tests`, add T36:

```rust
    #[test]
    fn the_migration_is_additive_and_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        // Build a round-3 database: today's schema without the round-4 columns.
        drop(StateDb::open(&path).unwrap());
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for (table, column) in [
                ("files", "held_write_id"), ("files", "held_base"), ("files", "held_version"),
                ("files", "held_object_version_id"), ("files", "version_filled"),
                ("operation_queue", "write_id"), ("operation_queue", "write_origin"),
                ("operation_queue", "after_write_id"), ("operation_queue", "base_pending"),
                ("operation_queue", "claim_id"), ("operation_queue", "claimed_at"),
                ("upload_resume", "completed_version"), ("upload_resume", "completed_object_version_id"),
            ] {
                let _ = conn.execute(&format!("DROP INDEX IF EXISTS idx_{table}_{column}"), []);
                conn.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), []).unwrap();
            }
            conn.execute("DROP TABLE id_aliases", []).unwrap();
            conn.execute(
                "INSERT INTO files (file_id, path, status, size_bytes, current_version) VALUES ('f1', 'a.txt', 'local', 7, 3)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO operation_queue (op_id, kind, file_id, base_version) VALUES ('op1', 'upload_version', 'f1', 3)",
                [],
            )
            .unwrap();
        }
        // Opened twice: the second open must not fail on an existing column.
        drop(StateDb::open(&path).unwrap());
        let db = StateDb::open(&path).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let columns = |table: &str| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1)).unwrap().map(Result::unwrap).collect()
        };
        for column in ["held_write_id", "held_base", "held_version", "held_object_version_id", "version_filled"] {
            assert!(columns("files").contains(&column.to_string()), "files.{column}");
        }
        for column in ["write_id", "write_origin", "after_write_id", "base_pending", "claim_id", "claimed_at"] {
            assert!(columns("operation_queue").contains(&column.to_string()), "operation_queue.{column}");
        }
        for column in ["completed_version", "completed_object_version_id"] {
            assert!(columns("upload_resume").contains(&column.to_string()), "upload_resume.{column}");
        }
        assert!(columns("id_aliases").contains(&"provisional_id".to_string()));

        // Rows are unchanged; the new columns read as their defaults.
        let row = db.get_file("f1").unwrap().unwrap();
        assert_eq!((row.path.as_str(), row.size_bytes), ("a.txt", 7));
        let presentation = db.item_presentation("f1").unwrap().unwrap();
        assert_eq!(presentation.contract.current_version, 3);
        assert!(presentation.held.is_none());
        assert!(!presentation.version_filled);
        assert_eq!(db.get_operation("op1").unwrap().unwrap().base_version, Some(3));
        assert!(db.finder_write("op1").unwrap().is_none(), "an op without a write id is not a Finder write");
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- write_token:: the_migration_is_additive_and_idempotent > $EVID/r4-t1-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t1-red.log | head
```

Expected: compile errors naming `mint_write_id`, `WriteToken`, `held_token`, `item_presentation` (the RED for new items is "not found").

- [ ] **Step 3: Implement `write_token.rs`**

Above the tests:

```rust
/// `W` is 32 lowercase hex digits (spec §5.1).
pub const WRITE_ID_HEX_LEN: usize = 32;

/// `{b}:w{W}`: `b` is the server version these bytes derive from, `W` the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteToken {
    pub base: i64,
    pub write_id: String,
}

impl WriteToken {
    pub fn render(&self) -> String {
        format!("{}:w{}", self.base, self.write_id)
    }
}

/// A fresh write id: a random v4 UUID without hyphens. Never derived from a name or path.
pub fn mint_write_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Strict parse of `^\d+:w[0-9a-f]{32}$`. Anything else is not a token.
pub fn parse_token(text: &str) -> Option<WriteToken> {
    let (base, rest) = text.split_once(':')?;
    if base.is_empty() || !base.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let write_id = rest.strip_prefix('w')?;
    if write_id.len() != WRITE_ID_HEX_LEN || !write_id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    Some(WriteToken { base: base.parse().ok()?, write_id: write_id.to_string() })
}

/// The `files.held_*` columns of one row (spec §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWrite {
    pub write_id: String,
    pub base: i64,
    /// The server version this write produced; `None` until it lands.
    pub version: Option<i64>,
    /// The object version this write produced; `None` until it lands.
    pub object_version_id: Option<String>,
}

impl HeldWrite {
    /// The token string; derived, never stored (spec §5.2).
    pub fn token(&self) -> String {
        WriteToken { base: self.base, write_id: self.write_id.clone() }.render()
    }
}

/// The predicate of spec §5.3. `held_write_queued`: an op with `write_id = held.write_id`
/// exists, in any state. Both inputs must come from one read (`StateDb::item_presentation`).
pub fn held_token(
    held: Option<&HeldWrite>,
    held_write_queued: bool,
    current_version: i64,
    current_object_version_id: Option<&str>,
) -> Option<String> {
    let held = held?;
    if held_write_queued {
        return Some(held.token());
    }
    let landed_here = held.version == Some(current_version)
        && matches!(
            (held.object_version_id.as_deref(), current_object_version_id),
            (Some(ours), Some(row)) if ours == row
        );
    landed_here.then(|| held.token())
}
```

Add `mod write_token;` after `mod state_db;` (`lib.rs:68`).

- [ ] **Step 4: Implement the migration and the one-read presentation in `state_db.rs`**

4.1 In `StateDb::open`, after `ensure_column(&conn, "files", "creator_for_fp", "TEXT")?;` (`SD:928`), add:

```rust
        // Task 1873 round 4 (spec 2026-10-09 §5.2): the write token. Additive and
        // nullable, no backfill; only the dedicated round-4 functions write them.
        ensure_column(&conn, "files", "held_write_id", "TEXT")?;
        ensure_column(&conn, "files", "held_base", "INTEGER")?;
        ensure_column(&conn, "files", "held_version", "INTEGER")?;
        ensure_column(&conn, "files", "held_object_version_id", "TEXT")?;
        ensure_column(&conn, "files", "version_filled", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "operation_queue", "write_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "write_origin", "TEXT")?;
        ensure_column(&conn, "operation_queue", "after_write_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "base_pending", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(&conn, "operation_queue", "claim_id", "TEXT")?;
        ensure_column(&conn, "operation_queue", "claimed_at", "INTEGER")?;
```

4.2 After the second `execute_batch` (it ends at `SD:966`, the `upload_resume` table), before the `INSERT OR IGNORE INTO staged_payloads` line (`SD:968`), add:

```rust
        ensure_column(&conn, "upload_resume", "completed_version", "INTEGER")?;
        ensure_column(&conn, "upload_resume", "completed_object_version_id", "TEXT")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_operation_queue_write_id ON operation_queue(write_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_after_write_id ON operation_queue(after_write_id);
            CREATE TABLE IF NOT EXISTS id_aliases (
                provisional_id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            ",
        )?;
```

4.3 Move the body of `get_file_contract_state` (`SD:2303-2335`) into `fn get_file_contract_state_conn(conn: &Connection, file_id: &str) -> Result<Option<FileContractState>>`; `get_file_contract_state` locks and calls it.

4.4 Add the types and readers (after `UploadResume`, `SD:713`):

```rust
/// Who queued a File Provider upload (plan Spec issue 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// Minted by the round-4 accept transaction: its bytes are the ones its token names.
    Minted,
    /// Queued by an earlier build, given a write id at engine start (spec §10.2).
    EarlierBuild,
}

impl WriteOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteOrigin::Minted => "minted",
            WriteOrigin::EarlierBuild => "earlier_build",
        }
    }

    pub fn from_db(value: Option<&str>) -> Option<Self> {
        match value? {
            "minted" => Some(WriteOrigin::Minted),
            "earlier_build" => Some(WriteOrigin::EarlierBuild),
            _ => None,
        }
    }
}

/// The round-4 columns of one File Provider upload op. `PendingOperation` is
/// deliberately unchanged (52 literal construction sites); these columns are read
/// and written only by the dedicated round-4 functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderWrite {
    pub write_id: String,
    pub origin: WriteOrigin,
    pub after_write_id: Option<String>,
    /// 0: the base is known. n >= 1: waiting for a snapshot, after n - 1
    /// successful snapshots that did not report the file (plan Spec issue 3).
    pub base_pending: i64,
}

/// Everything the payload builder needs, read in one locked call (spec §5.3).
#[derive(Debug, Clone)]
pub struct ItemPresentation {
    pub contract: FileContractState,
    pub held: Option<crate::write_token::HeldWrite>,
    /// An op carries `held.write_id`, in any state, parked included.
    pub held_write_queued: bool,
    /// A Finder upload of this file exists that has not parked (spec §9.1).
    pub unparked_finder_upload: bool,
    pub version_filled: bool,
}

fn finder_write_conn(conn: &Connection, op_id: &str) -> Result<Option<FinderWrite>> {
    conn.query_row(
        "SELECT write_id, write_origin, after_write_id, base_pending FROM operation_queue
         WHERE op_id = ?1 AND write_id IS NOT NULL",
        params![op_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )
    .optional()
    .map(|found| {
        found.map(|(write_id, origin, after_write_id, base_pending)| FinderWrite {
            write_id,
            // A write id without an origin can only come from a bug; treat it as
            // earlier-build so it is never handed over (spec §8.4, m-2).
            origin: WriteOrigin::from_db(origin.as_deref()).unwrap_or(WriteOrigin::EarlierBuild),
            after_write_id,
            base_pending,
        })
    })
}

fn held_write_conn(conn: &Connection, file_id: &str) -> Result<(Option<crate::write_token::HeldWrite>, bool)> {
    conn.query_row(
        "SELECT held_write_id, held_base, held_version, held_object_version_id, version_filled
         FROM files WHERE file_id = ?1",
        params![file_id],
        |row| {
            let write_id: Option<String> = row.get(0)?;
            let held = match write_id {
                Some(write_id) => Some(crate::write_token::HeldWrite {
                    write_id,
                    base: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    version: row.get(2)?,
                    object_version_id: row.get(3)?,
                }),
                None => None,
            };
            Ok((held, row.get::<_, i64>(4)? != 0))
        },
    )
}
```

and on `impl StateDb`:

```rust
    /// The predicate's inputs and the status override's, from one locked read
    /// (spec §5.3 "One read", §8.7 S1.9).
    pub fn item_presentation(&self, file_id: &str) -> Result<Option<ItemPresentation>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let Some(contract) = get_file_contract_state_conn(&tx, file_id)? else {
            return Ok(None);
        };
        let (held, version_filled) = held_write_conn(&tx, file_id)?;
        let held_write_queued = match &held {
            Some(held) => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
                params![held.write_id],
                |row| row.get(0),
            )?,
            None => false,
        };
        let unparked_finder_upload: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
              WHERE file_id = ?1 AND write_id IS NOT NULL
                AND kind IN ('upload_version', 'upload_file')
                AND attempts < max_attempts)",
            params![file_id],
            |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(Some(ItemPresentation { contract, held, held_write_queued, unparked_finder_upload, version_filled }))
    }

    pub fn finder_write(&self, op_id: &str) -> Result<Option<FinderWrite>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        finder_write_conn(&conn, op_id)
    }
```

- [ ] **Step 5: Run the tests; expect GREEN; then the mutations**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- write_token:: the_migration_is_additive_and_idempotent > $EVID/r4-t1-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/r4-t1-green.log
```

Expected: `ok. 3 passed; 0 failed`. Mutations, each RED then reverted:
- T7: `render` emits `self.base + 1` → `token_format_and_older_parse` fails at `parse_base_version_number(...) == Some(2)` (`$EVID/r4-t1-mut-T7.log`).
- P1: `matches!` arm also accepts `(None, _) | (_, None)` → `held_token_is_some_exactly_in_the_predicate_cases` fails at "m-3: the row's NULL never matches" (`$EVID/r4-t1-mut-P1.log`).
- T36: replace one `ensure_column(&conn, "files", "held_base", "INTEGER")?` with `conn.execute("ALTER TABLE files ADD COLUMN held_base INTEGER", [])?;` → the second `StateDb::open` fails with "duplicate column name" (`$EVID/r4-t1-mut-T36.log`).

- [ ] **Step 6: Gate and commit**

Run the task gate. Expected lib count: **`L0 + 3`**. Then:

```bash
cd $WT && git add -- src-tauri/src/write_token.rs src-tauri/src/lib.rs src-tauri/src/state_db.rs
git commit -m "feat(desktop): the write-token columns, the token helpers and a one-read item presentation" -- src-tauri/src/write_token.rs src-tauri/src/lib.rs src-tauri/src/state_db.rs
```

**Stop point:** after the commit. Report the RED and green lines, the three mutation logs and the gate counts. Do not start Task 2 until the lead has gated Task 1.

---

## Task 2: Serialization primitives: the claim, guarded writes, the release journal, test seams (spec §8.5, §8.7 S2–S4, S6; commit 2)

Every later rule uses these. Behaviour visible to a person changes in one way only: uploads of one file run in insertion order even when the clock steps back (M2).

**Files:**
- Modify: `src-tauri/src/state_db.rs`: queue order (`SD:2677`, `SD:3091`); remove `has_earlier_live_upload` (`SD:3097-3118`); add `ParkReason`, `ClaimOutcome`, `ClaimedOp`, `claim_operation`, the guarded writes, `finish_claimed`, `EngineStartRepair`, `engine_start_repair`, `set_created_at_for_test`
- Modify: `src-tauri/src/engine_bridge.rs`: `EngineBridge` (`EB:207-229`) and `new_with_stop_flag` (`EB:398-406`) gain `seams`; `process_due_operations` (`EB:462-539`); `execute_operation` (`EB:542-650`); `upload_version` / `do_upload_version` (`EB:651-890`); `finish_completed_upload` (`EB:998-1032`); Keep Mine's inline call (`EB:3193-3201`); the `VersionedServerMock` (`EB:12631-12863`)
- Modify: `src-tauri/src/runner.rs`: engine start (`RUN:997`)
- Test: `engine_bridge.rs` tests (next to `a_later_save_waits_for_an_earlier_save_that_is_backing_off`), `state_db.rs` tests

**Interfaces:**
- Consumes: `FinderWrite`, `finder_write_conn` (Task 1).
- Produces:
  - `pub enum ParkReason { StaleBase, BaseUnknown, PayloadMissing, PredecessorParked, PredecessorLost, RekeyFailed }`, `as_str()` → `"stale_base"`, `"base_unknown"`, `"payload_missing"`, `"predecessor_parked"`, `"predecessor_lost"`, `"rekey_failed"`
  - `pub struct ClaimedOp { pub op: PendingOperation, pub claim_id: String, pub write: Option<FinderWrite> }`
  - `pub enum ClaimOutcome { Gone, Wait, Claimed(ClaimedOp) }` (Task 3 adds `Parked`)
  - `StateDb::claim_operation(&self, op_id: &str, now: i64) -> Result<ClaimOutcome>`
  - `StateDb::record_attempt_claimed(&self, op_id: &str, claim_id: &str, attempts: i64, next_retry_at: i64, last_error: Option<&str>) -> Result<bool>`
  - `StateDb::record_pause_claimed(&self, op_id: &str, claim_id: &str, reason: OperationPauseReason, last_error: Option<&str>, now: i64) -> Result<bool>`
  - `StateDb::park_claimed(&self, op_id: &str, claim_id: &str, reason: ParkReason, now: i64) -> Result<bool>`
  - `StateDb::put_upload_resume_claimed(&self, resume: &UploadResume, claim_id: &str) -> Result<bool>`
  - `StateDb::finish_claimed(&self, op_id: &str, claim_id: &str, release_payload: Option<&str>) -> Result<bool>`
  - `pub struct EngineStartRepair { pub claims_cleared: usize, pub released_payloads: Vec<String> }`; `StateDb::engine_start_repair(&self) -> Result<EngineStartRepair>` (Tasks 6, 8, 9, 10 extend it)
  - `#[cfg(test)] StateDb::set_created_at_for_test(&self, op_id: &str, created_at: i64)`
  - every `bool` above: `true` = exactly one row matched; `false` = the op moved since the claim, nothing was written (S2)
  - engine: `pub(crate) struct QueueStateMoved` (an error type), `fn log_queue_state_moved(op_id: &str, step: &'static str)`; `#[cfg(test)] pub(crate) struct Seams` with `arm(&self, name: &'static str, hook: impl FnOnce() + Send + 'static)`; `fn seam(&self, name: &'static str)` on `EngineBridge` (a no-op outside tests). Seam names used in this plan: `"claim:before_tx"`, `"accept:before_tx"`, `"landing:after_complete"`.
  - `execute_operation(&self, claimed: &ClaimedOp, …) -> anyhow::Result<Option<String>>`: `Some(path)` is the staged payload to release after the op's removal commits (non-Windows uploads).
  - `upload_version(&self, op: &PendingOperation, claim: Option<&ClaimedOp>, sync_root: &Path, post_complete_errors: &mut Vec<String>) -> anyhow::Result<Option<String>>`. `claim: None` is Keep Mine's inline run (Spec issue 11): unguarded writes, as today.
  - mock: `VersionedServerState` gains `requests: Vec<(String, String)>` (method, path, every request) and `delay_init: Option<Duration>`.

- [ ] **Step 1: Write the failing tests**

In `engine_bridge.rs` tests:

```rust
    #[tokio::test]
    async fn m2_order_is_insertion_order_when_the_clock_steps_back() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [41u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "clock");
        queue_save(&bridge, dir.path(), "clock", "notes.txt", b"older save", "1");
        queue_save(&bridge, dir.path(), "clock", "notes.txt", b"older save, newer save", "1");
        // The wall clock stepped back between the two saves.
        let ops = bridge.db.list_operations_for_file("clock").unwrap();
        assert_eq!(ops.len(), 2);
        bridge.db.set_created_at_for_test(&ops[0].op_id, 2_000_000_000);
        bridge.db.set_created_at_for_test(&ops[1].op_id, 1_000_000_000);

        drain_upload_queue(&bridge, &sync_root).await;

        let state = server.finish();
        assert_eq!(state.files["clock"].versions.len(), 3, "{:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("clock", master_key),
            b"older save, newer save",
            "the newest bytes land last: {:?}",
            state.init_summary()
        );
    }

    #[tokio::test]
    async fn the_runners_copy_never_resurrects_or_drops_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [42u8; 32];
        let server = VersionedServerMock::start();
        server.state.lock().unwrap().delay_init = Some(Duration::from_millis(400));
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        seed_uploaded_row(&bridge, &server, "revoked");
        queue_save(&bridge, dir.path(), "revoked", "notes.txt", b"save under a share that goes away", "1");
        seed_uploaded_row(&bridge, &server, "kept");
        queue_save(&bridge, dir.path(), "kept", "other.txt", b"the second file's save", "1");
        let w = bridge.db.list_operations_for_file("revoked").unwrap().remove(0);
        // The first file is content of a share; the share is revoked while W's init is in flight.
        let mut contract = bridge.db.get_file_contract_state("revoked").unwrap().unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("revoked-root".into());
        bridge.db.set_file_contract_state(&contract).unwrap();

        let logs = capture_logs_async(async {
            let ((), ()) = tokio::join!(
                async {
                    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
                },
                async {
                    while server.state.lock().unwrap().inits.is_empty() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    // The revoked-share purge path (SD:2587-2620): a bulk delete by file id.
                    bridge.db.purge_revoked_shared_content(&[]).unwrap();
                }
            );
        })
        .await;

        let state = server.finish();
        assert!(bridge.db.get_upload_resume(&w.op_id).unwrap().is_none(), "no resume row for a purged op");
        assert!(bridge.db.get_operation(&w.op_id).unwrap().is_none(), "a purged op is never re-inserted");
        let moved: Vec<&str> = logs.lines().filter(|line| line.contains("queue state moved")).collect();
        assert_eq!(moved.len(), 1, "one line for the purged attempt:\n{logs}");
        assert!(moved[0].contains(&w.op_id) && moved[0].contains("resume"), "{logs}");
        assert_eq!(
            state.latest_plaintext("kept", master_key),
            b"the second file's save",
            "the other file's save is untouched and lands"
        );
    }
```

In `state_db.rs` tests:

```rust
    fn queued(op_id: &str, kind: OperationKind, file_id: &str, payload: Option<&str>) -> PendingOperation {
        PendingOperation {
            op_id: op_id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: payload.map(str::to_string),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn a_restore_and_an_upload_of_one_file_wait_for_each_other_in_queue_order() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", None)).unwrap();
        db.enqueue_operation(&queued("r", OperationKind::RestoreVersion, "f", None)).unwrap();
        db.enqueue_operation(&queued("u2", OperationKind::UploadVersion, "f", None)).unwrap();

        assert!(matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Wait), "a restore waits for an earlier upload");
        let ClaimOutcome::Claimed(u1) = db.claim_operation("u1", 1).unwrap() else { panic!("u1 is first") };
        assert!(db.finish_claimed("u1", &u1.claim_id, None).unwrap());
        assert!(matches!(db.claim_operation("u2", 1).unwrap(), ClaimOutcome::Wait), "an upload waits for an earlier restore");
        assert!(matches!(db.claim_operation("r", 1).unwrap(), ClaimOutcome::Claimed(_)));
        assert!(matches!(db.claim_operation("gone", 1).unwrap(), ClaimOutcome::Gone));
    }

    #[test]
    fn engine_start_clears_claims_and_releases_only_unreferenced_completed_payloads() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.enqueue_operation(&queued("u1", OperationKind::UploadVersion, "f", Some("/staged/referenced"))).unwrap();
        let ClaimOutcome::Claimed(stale) = db.claim_operation("u1", 1).unwrap() else { panic!("claimable") };
        db.track_staged_payload("/staged/free", None, true).unwrap();
        db.track_staged_payload("/staged/referenced", None, true).unwrap();
        db.track_staged_payload("/staged/unfinished", None, false).unwrap();

        let repair = db.engine_start_repair().unwrap();

        assert_eq!(repair.claims_cleared, 1);
        assert_eq!(repair.released_payloads, vec!["/staged/free".to_string()]);
        assert!(
            !db.record_attempt_claimed("u1", &stale.claim_id, 1, 0, None).unwrap(),
            "a claim from before the restart guards nothing"
        );
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- m2_order_is_insertion_order the_runners_copy_never_resurrects a_restore_and_an_upload_of_one_file engine_start_clears_claims > $EVID/r4-t2-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t2-red.log | head
```

Expected: compile errors for `set_created_at_for_test`, `delay_init`, `claim_operation`, `ClaimOutcome`, `finish_claimed`, `engine_start_repair`, `record_attempt_claimed`. Add those as `todo!()`-free stubs only if needed to reach a behavioural RED: T29 then fails "the newest bytes land last" (created_at order runs the newer save first) and T42 fails "no resume row for a purged op" (`put_upload_resume`, `SD:3152-3160`, upserts a row for the deleted op).

- [ ] **Step 3: The queue order (M2)**

- `list_due_operations` (`SD:2677`): `ORDER BY created_at ASC, rowid ASC` → `ORDER BY rowid ASC`.
- `list_operations_for_file` (`SD:3091`): the same change.
- Delete `has_earlier_live_upload` (`SD:3097-3118`); its only caller (`EB:494`) moves into the claim. Update the doc comment that names it (`EB:1213`) to name `StateDb::claim_operation`.

- [ ] **Step 4: The claim and the guarded writes in `state_db.rs`**

```rust
/// Why a File Provider upload parked with its bytes kept (spec §11, "Parked").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    StaleBase,
    BaseUnknown,
    PayloadMissing,
    PredecessorParked,
    PredecessorLost,
    RekeyFailed,
}

impl ParkReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ParkReason::StaleBase => "stale_base",
            ParkReason::BaseUnknown => "base_unknown",
            ParkReason::PayloadMissing => "payload_missing",
            ParkReason::PredecessorParked => "predecessor_parked",
            ParkReason::PredecessorLost => "predecessor_lost",
            ParkReason::RekeyFailed => "rekey_failed",
        }
    }
}

/// One op the runner may run now. Every later write for this attempt names `claim_id`
/// (spec §8.7 S2–S4).
#[derive(Debug, Clone)]
pub struct ClaimedOp {
    pub op: PendingOperation,
    pub claim_id: String,
    pub write: Option<FinderWrite>,
}

#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The op is gone.
    Gone,
    /// Not an attempt: an earlier content op of the same file is queued and has not parked.
    Wait,
    Claimed(ClaimedOp),
}

/// What engine start repaired (spec §8.7 S3, S6; §10.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartRepair {
    pub claims_cleared: usize,
    /// Journalled payloads marked released that no op, resume row or Windows
    /// finalization references: the caller unlinks them.
    pub released_payloads: Vec<String>,
}

/// Uploads and restores of one file run in insertion order (spec §8.5, m-9).
fn is_content_kind(kind: &OperationKind) -> bool {
    matches!(kind, OperationKind::UploadVersion | OperationKind::UploadFile | OperationKind::RestoreVersion)
}
```

On `impl StateDb`:

```rust
    /// S3: re-read the op, enforce the file's content order, and claim it, in one
    /// transaction. Replaces `get_operation` + `has_earlier_live_upload` (`EB:490-497`).
    pub fn claim_operation(&self, op_id: &str, now: i64) -> Result<ClaimOutcome> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(op) = tx
            .query_row(
                &format!("SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE op_id = ?1"),
                params![op_id],
                pending_operation_from_row,
            )
            .optional()?
        else {
            return Ok(ClaimOutcome::Gone);
        };
        if is_content_kind(&op.kind) {
            let earlier: bool = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM operation_queue AS this
                    JOIN operation_queue AS earlier
                      ON earlier.file_id = this.file_id AND earlier.op_id != this.op_id
                    WHERE this.op_id = ?1
                      AND earlier.kind IN ('upload_version', 'upload_file', 'restore_version')
                      AND earlier.attempts < earlier.max_attempts
                      AND earlier.rowid < this.rowid)",
                params![op_id],
                |row| row.get(0),
            )?;
            if earlier {
                return Ok(ClaimOutcome::Wait);
            }
        }
        let claim_id = uuid::Uuid::new_v4().simple().to_string();
        tx.execute(
            "UPDATE operation_queue SET claim_id = ?2, claimed_at = ?3 WHERE op_id = ?1",
            params![op_id, claim_id, now],
        )?;
        let write = finder_write_conn(&tx, op_id)?;
        tx.commit()?;
        Ok(ClaimOutcome::Claimed(ClaimedOp { op, claim_id, write }))
    }

    pub fn record_attempt_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        attempts: i64,
        next_retry_at: i64,
        last_error: Option<&str>,
    ) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE operation_queue
             SET attempts = ?3, next_retry_at = ?4, last_error = ?5, last_error_class = NULL,
                 paused_reason = NULL, updated_at = ?4, claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id, attempts, next_retry_at, last_error],
        )?;
        Ok(n == 1)
    }

    pub fn record_pause_claimed(
        &self,
        op_id: &str,
        claim_id: &str,
        reason: OperationPauseReason,
        last_error: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE operation_queue
             SET paused_reason = ?3, last_error_class = ?3, last_error = ?4, updated_at = ?5,
                 claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id, reason.as_str(), last_error.map(redact_diagnostic_error), now],
        )?;
        Ok(n == 1)
    }

    /// Park with the bytes kept: attempts used up, so nothing retries it (spec §8.4).
    pub fn park_claimed(&self, op_id: &str, claim_id: &str, reason: ParkReason, now: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE operation_queue
             SET attempts = max_attempts, last_error = ?3, last_error_class = ?3, updated_at = ?4,
                 claim_id = NULL, claimed_at = NULL
             WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id, reason.as_str(), now],
        )?;
        Ok(n == 1)
    }

    /// S4: the resume row is written only while the claimed op exists.
    pub fn put_upload_resume_claimed(&self, resume: &UploadResume, claim_id: &str) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "INSERT INTO upload_resume (
                op_id, payload_path, payload_size, payload_mtime_ns, upload_session_id,
                server_file_id, object_version_id, chunk_size_bytes, chunk_count,
                acked_chunks, metadata_applied, is_create, updated_at
             )
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%s','now')
             WHERE EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?13)
             ON CONFLICT(op_id) DO UPDATE SET
                payload_path = excluded.payload_path,
                payload_size = excluded.payload_size,
                payload_mtime_ns = excluded.payload_mtime_ns,
                upload_session_id = excluded.upload_session_id,
                server_file_id = excluded.server_file_id,
                object_version_id = excluded.object_version_id,
                chunk_size_bytes = excluded.chunk_size_bytes,
                chunk_count = excluded.chunk_count,
                acked_chunks = excluded.acked_chunks,
                metadata_applied = excluded.metadata_applied,
                is_create = excluded.is_create,
                updated_at = excluded.updated_at",
            params![
                resume.op_id, resume.payload_path, resume.payload_size, resume.payload_mtime_ns,
                resume.upload_session_id, resume.server_file_id, resume.object_version_id,
                resume.chunk_size_bytes, resume.chunk_count, resume.acked_chunks,
                resume.metadata_applied, resume.is_create, claim_id,
            ],
        )?;
        if n == 1 {
            tx.execute(
                "INSERT INTO staged_payloads(path, source_path, completed) VALUES (?1, NULL, 0)
                 ON CONFLICT(path) DO NOTHING",
                params![resume.payload_path],
            )?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// The op's removal after it succeeded, with its resume row; a released payload is
    /// marked in the journal in the same transaction, and unlinked by the caller only
    /// after this commits (S6).
    pub fn finish_claimed(&self, op_id: &str, claim_id: &str, release_payload: Option<&str>) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2",
            params![op_id, claim_id],
        )?;
        if n == 1 {
            tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
            if let Some(path) = release_payload {
                tx.execute(
                    "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
                     ON CONFLICT(path) DO UPDATE SET completed = 1",
                    params![path],
                )?;
            }
        }
        tx.commit()?;
        Ok(n == 1)
    }

    pub fn engine_start_repair(&self) -> Result<EngineStartRepair> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claims_cleared = tx.execute(
            "UPDATE operation_queue SET claim_id = NULL, claimed_at = NULL WHERE claim_id IS NOT NULL",
            [],
        )?;
        let released_payloads = {
            let mut stmt = tx.prepare(
                "SELECT path FROM staged_payloads
                 WHERE completed = 1
                   AND path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
                   AND path NOT IN (SELECT payload_path FROM upload_resume)
                   AND path NOT IN (SELECT payload_path FROM upload_finalizations)
                 ORDER BY path",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(EngineStartRepair { claims_cleared, released_payloads })
    }

    #[cfg(test)]
    pub(crate) fn set_created_at_for_test(&self, op_id: &str, created_at: i64) {
        let conn = self.0.lock().unwrap();
        conn.execute("UPDATE operation_queue SET created_at = ?2 WHERE op_id = ?1", params![op_id, created_at]).unwrap();
    }
```

- [ ] **Step 5: The runner loop on claims, the seams and the release after commit (`engine_bridge.rs`)**

5.1 Seams. Add after `pub struct EngineBridge` (`EB:229`):

```rust
/// Test-only hooks at named points where the plan's concurrency tests force an
/// interleaving (spec §13, "Concurrency tests"). Each hook fires once.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seams {
    hooks: std::sync::Mutex<std::collections::HashMap<&'static str, Box<dyn FnOnce() + Send>>>,
}

#[cfg(test)]
impl Seams {
    pub(crate) fn arm(&self, name: &'static str, hook: impl FnOnce() + Send + 'static) {
        self.hooks.lock().unwrap().insert(name, Box::new(hook));
    }

    fn fire(&self, name: &'static str) {
        let hook = self.hooks.lock().unwrap().remove(name);
        if let Some(hook) = hook {
            hook();
        }
    }
}
```

Field on `EngineBridge`: `#[cfg(test)] pub(crate) seams: Seams,`; in `new_with_stop_flag` (`EB:399-405`): `#[cfg(test)] seams: Seams::default(),`. Method:

```rust
    /// A named point for the concurrency tests; nothing outside tests.
    fn seam(&self, #[cfg_attr(not(test), allow(unused_variables))] name: &'static str) {
        #[cfg(test)]
        self.seams.fire(name);
    }
```

Test helper in the tests module, used by Tasks 3, 4, 5 and 7:

```rust
    /// Run `action` on its own thread and wait for it, at most 500 ms (spec §13: a
    /// correct implementation may block the action on the state-db mutex).
    fn run_competing(action: impl FnOnce() + Send + 'static) {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            action();
            let _ = done.send(());
        });
        let _ = wait.recv_timeout(Duration::from_millis(500));
    }
```

5.2 The error type and its log line (near `log_refused_upload`, `EB:4004`):

```rust
/// A guarded queue write matched no row: the op moved since its claim (spec §8.7 S2, S4).
#[derive(Debug)]
pub(crate) struct QueueStateMoved;

impl std::fmt::Display for QueueStateMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("queue state moved")
    }
}

impl std::error::Error for QueueStateMoved {}

fn log_queue_state_moved(op_id: &str, step: &'static str) {
    tracing::warn!(op_id = %op_id, step, "queue state moved");
}
```

5.3 Replace the loop body of `process_due_operations` from `let Some(op) = self.db.get_operation(&op.op_id)? else {` (`EB:490`) to the end of the `match result` (`EB:535`) with:

```rust
            self.seam("claim:before_tx");
            let claimed = match self.db.claim_operation(&op.op_id, now)? {
                ClaimOutcome::Gone | ClaimOutcome::Wait => continue,
                ClaimOutcome::Claimed(claimed) => claimed,
            };
            let op = claimed.op.clone();
            let result = self
                .execute_operation(&claimed, sync_root, now, &mut outcome.post_complete_errors)
                .await;
            match result {
                Ok(release) => {
                    if !self.db.finish_claimed(&op.op_id, &claimed.claim_id, release.as_deref())? {
                        log_queue_state_moved(&op.op_id, "landing");
                        continue;
                    }
                    if let Some(path) = release.as_deref()
                        && let Err(e) = crate::staged_payload::remove(&self.db, Path::new(path))
                    {
                        tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
                    }
                    if let Some(file_id) = &op.file_id {
                        outcome.invalidated_item_ids.push(file_id.clone());
                    }
                    outcome.completed_op_ids.push(op.op_id);
                }
                Err(error) if error.downcast_ref::<QueueStateMoved>().is_some() => continue,
                Err(error) => {
                    let class = classify_operation_error(&error.to_string());
                    if let Some(reason) = class.pause_reason() {
                        if !self.db.record_pause_claimed(&op.op_id, &claimed.claim_id, reason, Some(&error.to_string()), now)? {
                            log_queue_state_moved(&op.op_id, "pause");
                            continue;
                        }
                        outcome.paused_op_ids.push(op.op_id);
                    } else {
                        let attempts = op.attempts.saturating_add(1);
                        if matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                            && error_http_status(&error) == Some(409)
                        {
                            log_refused_upload(&op, attempts);
                        }
                        let next_retry_at = now.saturating_add(retry_delay_seconds(attempts));
                        if !self.db.record_attempt_claimed(&op.op_id, &claimed.claim_id, attempts, next_retry_at, Some(&error.to_string()))? {
                            log_queue_state_moved(&op.op_id, "attempt");
                            continue;
                        }
                        if attempts >= op.max_attempts {
                            self.abandon_upload_after_give_up(&op).await;
                        }
                        outcome.retried_op_ids.push(op.op_id);
                    }
                }
            }
```

5.4 `execute_operation` takes `claimed: &ClaimedOp` (use `&claimed.op` where it used `op`) and returns `anyhow::Result<Option<String>>`: every non-upload arm returns `Ok(None)`; the upload arm (`EB:645-647`) is `self.upload_version(&claimed.op, Some(claimed), sync_root, post_complete_errors).await`.

5.5 `upload_version` / `do_upload_version` gain `claim: Option<&ClaimedOp>` and return `anyhow::Result<Option<String>>`. At the resume write (`EB:798`):

```rust
                match claim {
                    Some(claim) => {
                        if !self.db.put_upload_resume_claimed(&session, &claim.claim_id)? {
                            log_queue_state_moved(&op.op_id, "resume");
                            return Err(anyhow::Error::new(QueueStateMoved));
                        }
                    }
                    // Keep Mine runs inline, outside the queue (plan Spec issue 11).
                    None => self.db.put_upload_resume(&session)?,
                }
```

The success arm's final `Ok(())` (`EB:856`) becomes `Ok(released)` where `let released = if cfg!(target_os = "windows") { None } else { Some(payload_path_str.clone()) };`.

5.6 `finish_completed_upload` (`EB:1028-1030`): wrap the `crate::staged_payload::remove` call in `#[cfg(target_os = "windows")] { … }`. On Windows nothing changes; elsewhere the payload is released by `finish_claimed` and unlinked after it commits (5.3).

5.7 Keep Mine (`EB:3193-3201`): `self.upload_version(&op, None, sync_root, &mut post_complete_errors).await` returns `Ok(released)`; on `Ok(Some(path))` call `crate::staged_payload::remove(&self.db, Path::new(&path))` with the same warn on failure (today `finish_completed_upload` unlinked it).

5.8 Mock (`EB:12631-12650`, `EB:12750-12762`): add `requests: Vec<(String, String)>` and `delay_init: Option<Duration>` to `VersionedServerState`. In `versioned_server_response`, push `(request.method.clone(), request.path.clone())` to `s.requests` first, and return `s.delay_init` as the delay when `request.method == "POST" && request.path == "/api/v1/uploads/init"` (else the existing chunk delay).

- [ ] **Step 6: Engine start (`runner.rs:997`)**

Before `match db.reconcile_stale_in_flight_on_startup() {`:

```rust
    match db.engine_start_repair() {
        Ok(repair) => {
            for path in &repair.released_payloads {
                if let Err(e) = crate::staged_payload::remove(&db, std::path::Path::new(path)) {
                    tracing::warn!(error = %e, "released upload copy kept; removal is retried at the next start");
                }
            }
            if repair.claims_cleared > 0 || !repair.released_payloads.is_empty() {
                tracing::info!(
                    claims_cleared = repair.claims_cleared,
                    released = repair.released_payloads.len(),
                    "engine start: queue claims cleared, released upload copies removed"
                );
            }
        }
        Err(e) => tracing::warn!(error = %e, "engine start repair failed"),
    }
```

- [ ] **Step 7: GREEN, then the mutations**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- m2_order_is_insertion_order the_runners_copy_never_resurrects a_restore_and_an_upload_of_one_file engine_start_clears_claims > $EVID/r4-t2-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/r4-t2-green.log
```

Expected `ok. 4 passed`. Mutations:
- T29: `list_due_operations` back to `ORDER BY created_at ASC, rowid ASC` and the claim's `earlier.rowid < this.rowid` to `earlier.created_at < this.created_at` → "the newest bytes land last" fails (`$EVID/r4-t2-mut-T29.log`).
- T42: at 5.5, call `self.db.put_upload_resume(&session)?` for every claim → "no resume row for a purged op" fails (`$EVID/r4-t2-mut-T42.log`).
- P2: drop `'restore_version'` from the claim's `IN` list and `RestoreVersion` from `is_content_kind` → "a restore waits for an earlier upload" fails (`$EVID/r4-t2-mut-P2.log`).
- P3: drop the `operation_queue` line from the released-payload query → the `released_payloads` assertion fails with `/staged/referenced` included (`$EVID/r4-t2-mut-P3.log`).

- [ ] **Step 8: Gate and commit**

Task gate; expected lib count **`L0 + 7`**. Every Keep Mine test and every round-3 queue test must stay green unchanged.

```bash
cd $WT && git commit -m "feat(desktop): claim queue ops in one transaction, guard the runner's writes, release staged copies after commit" -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/runner.rs
```

**Stop point:** after the commit; report counts and the four mutation logs.

---

## Task 3: The accept transaction and the base mapping (spec §6.1, §6.3.1, §8.1, §8.3, §8.7 S1.1, S3, S5; commit 3)

The token is minted and the base is mapped through it, but replies still report `{cv}` (Task 4 switches them). Every existing round-3 test stays green, because the mapping handles `{cv}` bases through rules 3–5.

**Files:**
- Modify: `src-tauri/src/write_token.rs`: `BaseFacts`, `BaseDecision`, `decide_base`
- Modify: `src-tauri/src/state_db.rs`: connection-level twins `get_file_conn`, `upsert_file_conn`, `record_local_write_conn` (bodies of `SD:1047`, `SD:974-1046`, `SD:2010-2031`, whose public methods then lock and call them); `FinderAccept`, `FinderAcceptKind`, `AcceptOutcome`, `accept_finder_write`; `carry_held_write`, `set_held_landed`, `resolve_successors`; the claim's steps 3–4; `ClaimOutcome::Parked`; test-only `set_base_version_for_test`
- Modify: `src-tauri/src/engine_bridge.rs`: `FpWrite`; `queue_file_provider_create_from` / `queue_file_provider_modify_from` (+ the `contents: None` wrappers); `legacy_item_identifiers` becomes `pub(crate)` (`EB:4710`); `ParkNow` and `log_parked`; `upload_init_request_for_operation` (`EB:3640-3662`) gains the guard; the landing (`EB:815-833`, `EB:1121-1194`, `EB:1216-1258`)
- Modify: `src-tauri/src/ipc_socket.rs`: the create and modify arms (`IPC:1832`, `IPC:1877`) call the new entry points; `write_outcome_response` (`IPC:2364-2396`) takes `anyhow::Result<FpWrite>`; the delete arm (`IPC:1886-1900`) wraps with `.map(FpWrite::plain)`
- Test: `engine_bridge.rs` tests; the Finder tests of the versioned section switch entry points (step 1.1)

**Interfaces:**
- Consumes: `HeldWrite`, `mint_write_id`, `WriteToken` (Task 1); `claim_operation`, `ClaimedOp`, `ParkReason`, `park_claimed`, seams, `run_competing` (Task 2).
- Produces:
  - `write_token::BaseFacts { current_version: i64, version_filled: bool, held: Option<HeldWrite>, held_write_queued: bool, create_queued: bool, minted_resolved_bases: Vec<i64>, legacy_identifiers: Vec<String> }` (`Default`)
  - `write_token::BaseDecision { After { write_id: String, b: i64 }, Resolved { base: i64 }, Pending, ParkUnknown }`, `BaseDecision::token_base(&self) -> i64`
  - `write_token::decide_base(facts: &BaseFacts, incoming: Option<&str>) -> BaseDecision`
  - `state_db::FinderAccept<'a> { op_id, file_id, kind: FinderAcceptKind<'a>, parent_id: Option<&'a str>, target_path: Option<&'a str>, metadata_json: &'a str, payload_path: &'a str, size_bytes: i64, modified_at: i64, backup_source_key: Option<&'a str>, now: i64 }`
  - `state_db::FinderAcceptKind<'a> { Create { row: &'a FileEntry }, Modify { incoming_base: Option<&'a str> } }`
  - `state_db::AcceptOutcome { Queued { token: String, decision: Option<BaseDecision> }, ParkedAtOnce { token: String, reason: ParkReason }, UnknownItem }`
  - `StateDb::accept_finder_write(&self, accept: &FinderAccept<'_>, legacy_identifiers: &dyn Fn(&FileEntry, &FileContractState) -> Vec<String>) -> Result<AcceptOutcome>`
  - `StateDb::carry_held_write(&self, from_file_id: &str, to_file_id: &str) -> Result<()>`, `StateDb::set_held_landed(&self, file_id: &str, write_id: &str, version: i64, object_version_id: Option<&str>) -> Result<bool>`, `StateDb::resolve_successors(&self, write_id: &str, version: i64, object_version_id: Option<&str>) -> Result<usize>` (Task 7 folds all three into the landing transaction)
  - `ClaimOutcome::Parked { op_id: String, file_id: Option<String>, reason: ParkReason }`
  - `engine_bridge::FpWrite { pub outcome: FinderWriteOutcome, pub token: Option<String> }`, `FpWrite::plain(FinderWriteOutcome) -> FpWrite` (Task 8 adds `present_as`)
  - `EngineBridge::queue_file_provider_create_from(&self, target: FinderWriteTarget, contents: Option<&std::fs::File>) -> anyhow::Result<FpWrite>`; `queue_file_provider_modify_from` (same shape); `queue_file_provider_create(target)` / `queue_file_provider_modify(target)` pass `None`
  - `pub(crate) struct ParkNow(pub ParkReason)` (an error type the runner parks on); `fn log_parked(op_id: &str, file_id: Option<&str>, reason: ParkReason)` → `warn!` "upload parked with its bytes kept in the queue" with `op_id`, `file_id`, `reason`
  - test helpers: `fp_save(bridge, dir, file_id, filename, bytes, base) -> FpWrite`, `fp_create(bridge, dir, filename, bytes) -> FpWrite`; `queue_save` (`EB:13168`) becomes a wrapper of `fp_save`

- [ ] **Step 1: Write the failing tests**

1.1 Switch the Finder tests to the File Provider entry points first (they describe Finder saves; spec §8.8 keeps minting off `queue_finder_*`). In `engine_bridge.rs` tests from `struct MockServerFile` (`EB:12631`) to the end of the module, replace `bridge.queue_finder_create(` with `bridge.queue_file_provider_create(` and `bridge.queue_finder_modify(` with `bridge.queue_file_provider_modify(` (append `.outcome` where a test reads the returned outcome). Add the helpers next to `queue_save`:

```rust
    fn fp_save(bridge: &EngineBridge, dir: &Path, file_id: &str, filename: &str, bytes: &[u8], base: &str) -> FpWrite {
        let contents = dir.join(format!("save-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge
            .queue_file_provider_modify(finder_file_target(Some(file_id), filename, &contents, Some(base.to_string())))
            .unwrap()
    }

    fn fp_create(bridge: &EngineBridge, dir: &Path, filename: &str, bytes: &[u8]) -> FpWrite {
        let contents = dir.join(format!("create-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&contents, bytes).unwrap();
        bridge.queue_file_provider_create(finder_file_target(None, filename, &contents, None)).unwrap()
    }
```

and make `queue_save` (`EB:13168-13179`) call `fp_save(...)` and drop the result.

1.2 The new tests (`json!` is `serde_json::json!`; add `use serde_json::json;` to the tests module if it is missing). All are `#[tokio::test]`, each with `let dir = tempfile::tempdir().unwrap(); let sync_root = dir.path().join("sync-root"); std::fs::create_dir_all(&sync_root).unwrap();`, its own `master_key` byte and a `VersionedServerMock`; shown from the first line that differs):

```rust
    #[tokio::test]
    async fn i2_a_zero_base_on_a_versioned_row_parks() {
        // setup as above, master_key [44u8; 32]
        seed_uploaded_row(&bridge, &server, "zero-base");
        fp_save(&bridge, dir.path(), "zero-base", "notes.txt", b"edit on a stale 0", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(state.inits.is_empty(), "nothing is sent: {:?}", state.init_summary());
        let op = bridge.db.list_operations_for_file("zero-base").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked at once");
        assert!(std::path::Path::new(op.payload_path.as_deref().unwrap()).is_file(), "the bytes are kept");
    }

    #[tokio::test]
    async fn the_init_guard_refuses_a_finder_replace_without_a_base() {
        // setup as above, master_key [45u8; 32]
        seed_uploaded_row(&bridge, &server, "too-big");
        fp_save(&bridge, dir.path(), "too-big", "notes.txt", b"a base beyond i32", "3000000000");
        seed_uploaded_row(&bridge, &server, "no-base");
        fp_save(&bridge, dir.path(), "no-base", "notes.txt", b"a base that went missing", "1");
        let op = bridge.db.list_operations_for_file("no-base").unwrap().remove(0);
        bridge.db.set_base_version_for_test(&op.op_id, None);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert!(state.inits.is_empty(), "no replace without a base: {:?}", state.init_summary());
        for file in ["too-big", "no-base"] {
            let op = bridge.db.list_operations_for_file(file).unwrap().remove(0);
            assert_eq!(op.attempts, op.max_attempts, "{file} parked");
        }
    }

    #[tokio::test]
    async fn a_newer_save_queues_behind_a_write_without_a_session_and_both_land() {
        // setup as above, master_key [46u8; 32]
        seed_uploaded_row(&bridge, &server, "two");
        let first = fp_save(&bridge, dir.path(), "two", "notes.txt", b"first", "1");
        fp_save(&bridge, dir.path(), "two", "notes.txt", b"first, second", first.token.as_deref().unwrap_or("1"));
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("two"), json!(1), 201), (json!("two"), json!(2), 201)],
            "one upload per save, in order"
        );
        assert_eq!(state.files["two"].versions.len(), 3, "both saves are versions in the history");
        assert_eq!(state.latest_plaintext("two", master_key), b"first, second");
    }

    #[tokio::test]
    async fn a_newer_save_waits_behind_a_live_session() {
        // setup as above, master_key [47u8; 32]
        server.state.lock().unwrap().delay_chunks.insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "live");
        let first = fp_save(&bridge, dir.path(), "live", "notes.txt", b"first", "1");
        let first_token = first.token.clone().unwrap();
        let ((), ()) = tokio::join!(
            async { drain_upload_queue(&bridge, &sync_root).await; },
            async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                fp_save(&bridge, dir.path(), "live", "notes.txt", b"first, second", &first_token);
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("live"), json!(1), 201), (json!("live"), json!(2), 201)],
            "the second save is based on what the first produced"
        );
        assert_eq!(state.latest_plaintext("live", master_key), b"first, second");
    }

    #[tokio::test]
    async fn enqueue_vs_runner_a_save_accepted_while_its_predecessor_lands_is_never_orphaned() {
        // setup as above, master_key [48u8; 32]; the bridge is shared with the competing thread:
        let bridge = Arc::new(test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key));
        seed_uploaded_row(&bridge, &server, "race");
        let w = fp_save(&bridge, dir.path(), "race", "notes.txt", b"W", "1");
        let w_token = w.token.clone().unwrap();
        // N's accept stops at its seam; W's landing commits meanwhile (review sequence C).
        let runner = Arc::clone(&bridge);
        let root = sync_root.clone();
        bridge.seams.arm("accept:before_tx", move || {
            run_competing(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async { runner.process_due_operations(&root, now_secs()).await.unwrap(); });
            });
        });
        let logs = capture_logs_async(async {
            fp_save(&bridge, dir.path(), "race", "notes.txt", b"W, then N", &w_token);
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("race"), json!(1), 201), (json!("race"), json!(2), 201)],
            "N is based on W's produced version"
        );
        assert_eq!(state.latest_plaintext("race", master_key), b"W, then N");
        assert!(!logs.contains("predecessor_lost"), "{logs}");
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(), "0 ops left");
    }

    #[tokio::test]
    async fn a_numeric_base_while_a_minted_write_is_queued_follows_the_newest_write() {
        // setup as above, master_key [49u8; 32]
        seed_uploaded_row(&bridge, &server, "chain");
        let w1 = fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1", "1");
        fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1 2", w1.token.as_deref().unwrap());
        // The system sent this save before it recorded either reply.
        fp_save(&bridge, dir.path(), "chain", "notes.txt", b"1 2 3", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("chain"), json!(1), 201), (json!("chain"), json!(2), 201), (json!("chain"), json!(3), 201)],
            "the third save follows the newest write of the chain"
        );
        assert_eq!(state.latest_plaintext("chain", master_key), b"1 2 3");
    }

    #[tokio::test]
    async fn an_unknown_token_is_sent_as_its_b() {
        // setup as above, master_key [50u8; 32]
        seed_uploaded_row(&bridge, &server, "unknown-token");
        server.seed_file("unknown-token", 3);
        let mut contract = bridge.db.get_file_contract_state("unknown-token").unwrap().unwrap();
        contract.current_version = 3;
        bridge.db.set_file_contract_state(&contract).unwrap();
        fp_save(&bridge, dir.path(), "unknown-token", "notes.txt", b"edit", &format!("3:w{}", "c".repeat(32)));
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("unknown-token"), json!(3), 201)]);
    }

    #[tokio::test]
    async fn a_waiting_op_stores_its_b_as_base_version() {
        // setup as above, master_key [51u8; 32] (m-4a: an older build would send this, and the server refuses it)
        seed_uploaded_row(&bridge, &server, "stores-b");
        let w1 = fp_save(&bridge, dir.path(), "stores-b", "notes.txt", b"1", "1");
        fp_save(&bridge, dir.path(), "stores-b", "notes.txt", b"1 2", w1.token.as_deref().unwrap());
        let ops = bridge.db.list_operations_for_file("stores-b").unwrap();
        assert_eq!(ops[1].base_version, Some(1), "after W1, stored as W1's b");
        let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(&bridge, dir.path(), &provisional, "new.txt", b"created, edited", created.token.as_deref().unwrap());
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops[1].base_version, Some(0), "after a create, stored as 0");
        drop(server.finish());
    }
```

(`FpWrite::outcome_file_id(&self) -> String` is a small test-only accessor returning the `file_id` of `FinderWriteOutcome::Queued`.)

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- i2_a_zero_base_on_a_versioned_row_parks the_init_guard_refuses a_newer_save_queues_behind a_newer_save_waits_behind enqueue_vs_runner a_numeric_base_while a_minted an_unknown_token_is_sent a_waiting_op_stores > $EVID/r4-t3-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t3-red.log | head -20
```

Expected first: compile errors (`queue_file_provider_modify`, `FpWrite`, `set_base_version_for_test`). With the entry points stubbed to call today's `queue_finder_*` and return `token: None`, the behavioural REDs are: T12 sends a replace without a base; T14 sends both; T39 sends N on base 1 after W made v2, `(race,1,409)`; T49 sends the third save on base 1, `409`. T21, T22 and T50 are guards and may already pass (spec T21, T50); T22's RED comes from its mutation.

- [ ] **Step 3: `decide_base` in `write_token.rs`**

```rust
/// What the accept transaction knows before this write changes the row (spec §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseFacts {
    pub current_version: i64,
    /// Rule 6a. Set by the snapshot fill (Task 6); false until then.
    pub version_filled: bool,
    pub held: Option<HeldWrite>,
    /// An op carries `held.write_id`, in any state.
    pub held_write_queued: bool,
    /// A create of this file is queued, in any state: the row is provisional (plan Spec issue 4).
    pub create_queued: bool,
    /// Resolved bases of this file's queued uploads minted by this code.
    pub minted_resolved_bases: Vec<i64>,
    /// What a build before the server-version-led format reported for the row now (`EB:4710-4716`).
    pub legacy_identifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseDecision {
    /// Rules 1a, 2, 3: the successor of the queued write `write_id`; `b` is that write's `b`.
    After { write_id: String, b: i64 },
    /// Rules 1b, 4, 5, 6a: the server version this upload replaces.
    Resolved { base: i64 },
    /// Rule 6b: wait for a snapshot to learn the version.
    Pending,
    /// Rule 6a′: queued and parked at once, bytes kept.
    ParkUnknown,
}

impl BaseDecision {
    /// `b` of the new write's token (spec §5.1).
    pub fn token_base(&self) -> i64 {
        match self {
            BaseDecision::After { b, .. } => *b,
            BaseDecision::Resolved { base } => *base,
            BaseDecision::Pending | BaseDecision::ParkUnknown => 0,
        }
    }
}

/// `{v}` or `{v}:{hash}`, never a token or a three-segment identifier (rule 3).
fn content_version_number(identifier: &str) -> Option<i64> {
    if parse_token(identifier).is_some() {
        return None;
    }
    let mut parts = identifier.split(':');
    let version = parts.next()?.parse::<i64>().ok().filter(|v| *v > 0)?;
    let _hash = parts.next();
    parts.next().is_none().then_some(version)
}

/// Spec §6.1: the first matching row wins. Rule 1c is split off (§12): a token that is
/// not the held one falls to rule 5.
pub fn decide_base(facts: &BaseFacts, incoming: Option<&str>) -> BaseDecision {
    let newest = facts.held.as_ref().filter(|_| facts.held_write_queued);
    let after = |held: &HeldWrite| BaseDecision::After { write_id: held.write_id.clone(), b: held.base };
    if let (Some(held), Some(incoming)) = (facts.held.as_ref(), incoming)
        && incoming == held.token()
    {
        if facts.held_write_queued {
            return after(held); // 1a
        }
        if let Some(version) = held.version {
            return BaseDecision::Resolved { base: version }; // 1b
        }
    }
    if facts.create_queued
        && let Some(held) = newest
    {
        return after(held); // 2
    }
    if let (Some(v), Some(held)) = (incoming.and_then(content_version_number), newest)
        && facts.minted_resolved_bases.contains(&v)
    {
        return after(held); // 3
    }
    if let Some(incoming) = incoming
        && facts.current_version > 0
        && facts.legacy_identifiers.iter().any(|legacy| legacy == incoming)
    {
        if let Some(held) = newest.filter(|_| facts.minted_resolved_bases.contains(&facts.current_version)) {
            return after(held); // 4, then 3
        }
        return BaseDecision::Resolved { base: facts.current_version }; // 4
    }
    if let Some(v) = crate::engine_bridge::parse_base_version_number(incoming) {
        return BaseDecision::Resolved { base: v }; // 5
    }
    if facts.current_version > 0 {
        if facts.version_filled {
            BaseDecision::Resolved { base: facts.current_version } // 6a
        } else {
            BaseDecision::ParkUnknown // 6a′
        }
    } else {
        BaseDecision::Pending // 6b
    }
}
```

- [ ] **Step 4: The accept transaction in `state_db.rs`**

4.1 Extract `get_file_conn`, `upsert_file_conn` and `record_local_write_conn` from the bodies of `get_file` (`SD:1047`), `upsert_file` (`SD:974-1046`) and `record_local_write` (`SD:2010-2031`), generic over `C: std::ops::Deref<Target = Connection>` like `record_file_change_conn` (`SD:387-391`), so a `Transaction` can be passed. The public methods lock and call them; behaviour, including the change-log rows they record, is unchanged.

4.2 The types (`FinderAccept`, `FinderAcceptKind`, `AcceptOutcome`) as in Interfaces, and:

```rust
fn base_facts_conn(
    conn: &Connection,
    file_id: &str,
    contract: &FileContractState,
    legacy_identifiers: Vec<String>,
) -> Result<crate::write_token::BaseFacts> {
    let (held, version_filled) = held_write_conn(conn, file_id)?;
    let held_write_queued = match &held {
        Some(held) => conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
            params![held.write_id],
            |row| row.get(0),
        )?,
        None => false,
    };
    let create_queued: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM operation_queue
          WHERE file_id = ?1 AND kind IN ('upload_version', 'upload_file')
            AND json_extract(metadata_json, '$.operation') = 'create_file')",
        params![file_id],
        |row| row.get(0),
    )?;
    let minted_resolved_bases = {
        let mut stmt = conn.prepare(
            "SELECT base_version FROM operation_queue
             WHERE file_id = ?1 AND write_origin = 'minted' AND after_write_id IS NULL
               AND base_pending = 0 AND base_version IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![file_id], |row| row.get::<_, i64>(0))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    Ok(crate::write_token::BaseFacts {
        current_version: contract.current_version,
        version_filled,
        held,
        held_write_queued,
        create_queued: create_queued && contract.current_version == 0,
        minted_resolved_bases,
        legacy_identifiers,
    })
}
```

4.3 `accept_finder_write` (S1.1: one `BEGIN IMMEDIATE`; nothing inside awaits):

```rust
    pub fn accept_finder_write(
        &self,
        accept: &FinderAccept<'_>,
        legacy_identifiers: &dyn Fn(&FileEntry, &FileContractState) -> Vec<String>,
    ) -> Result<AcceptOutcome> {
        use crate::write_token::{BaseDecision, WriteToken, decide_base, mint_write_id};
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (decision, object_version_id) = match &accept.kind {
            FinderAcceptKind::Create { row } => {
                upsert_file_conn(&tx, row)?;
                (None, None)
            }
            FinderAcceptKind::Modify { incoming_base } => {
                let (Some(entry), Some(contract)) =
                    (get_file_conn(&tx, accept.file_id)?, get_file_contract_state_conn(&tx, accept.file_id)?)
                else {
                    return Ok(AcceptOutcome::UnknownItem);
                };
                let facts = base_facts_conn(&tx, accept.file_id, &contract, legacy_identifiers(&entry, &contract))?;
                let decision = decide_base(&facts, *incoming_base);
                record_local_write_conn(&tx, accept.file_id, accept.size_bytes, accept.modified_at)?;
                (Some(decision), contract.current_object_version_id.clone())
            }
        };
        let write_id = mint_write_id();
        let b = decision.as_ref().map_or(0, BaseDecision::token_base);
        tx.execute(
            "UPDATE files SET held_write_id = ?2, held_base = ?3, held_version = NULL,
                              held_object_version_id = NULL
             WHERE file_id = ?1",
            params![accept.file_id, write_id, b],
        )?;
        // m-4a: a waiting op stores its token's b, so an older build sends a base the
        // server refuses, never none.
        let (base_version, after_write_id, base_pending, parked) = match &decision {
            None => (None, None, 0_i64, false),
            Some(BaseDecision::After { write_id, b }) => (Some(*b), Some(write_id.clone()), 0, false),
            Some(BaseDecision::Resolved { base }) => (Some(*base), None, 0, false),
            Some(BaseDecision::Pending) => (Some(0), None, 1, false),
            Some(BaseDecision::ParkUnknown) => (Some(0), None, 0, true),
        };
        tx.execute(
            "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                last_error, last_error_class, paused_reason, backup_source_key, created_at, updated_at,
                write_id, write_origin, after_write_id, base_pending
             ) VALUES (?1, 'upload_version', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 25, ?10, ?11, ?12, NULL, ?13,
                       ?10, ?10, ?14, 'minted', ?15, ?16)",
            params![
                accept.op_id,
                accept.file_id,
                accept.parent_id,
                accept.target_path,
                accept.metadata_json,
                accept.payload_path,
                base_version,
                object_version_id,
                if parked { 25 } else { 0 },
                accept.now,
                if parked { "base_unknown" } else { "queued from Finder; upload worker not yet attached" },
                parked.then_some("base_unknown"),
                accept.backup_source_key,
                write_id,
                after_write_id,
                base_pending,
            ],
        )?;
        tx.commit()?;
        let token = WriteToken { base: b, write_id }.render();
        Ok(if parked {
            AcceptOutcome::ParkedAtOnce { token, reason: ParkReason::BaseUnknown }
        } else {
            AcceptOutcome::Queued { token, decision }
        })
    }
```

4.4 The held columns at the landing and the chain step by write id:

```rust
    /// A create's id swap: S takes P's held columns (spec §5.4 row 14). Task 7 folds this
    /// into the landing transaction.
    pub fn carry_held_write(&self, from_file_id: &str, to_file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET
                held_write_id = (SELECT held_write_id FROM files WHERE file_id = ?1),
                held_base = (SELECT held_base FROM files WHERE file_id = ?1),
                held_version = (SELECT held_version FROM files WHERE file_id = ?1),
                held_object_version_id = (SELECT held_object_version_id FROM files WHERE file_id = ?1)
             WHERE file_id = ?2
               AND EXISTS (SELECT 1 FROM files WHERE file_id = ?1 AND held_write_id IS NOT NULL)",
            params![from_file_id, to_file_id],
        )?;
        Ok(())
    }

    /// §8.6.2: only `WHERE held_write_id = W`; a later save's token stays held (§5.4 row 6).
    pub fn set_held_landed(&self, file_id: &str, write_id: &str, version: i64, object_version_id: Option<&str>) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE files SET held_version = ?3, held_object_version_id = ?4
             WHERE file_id = ?1 AND held_write_id = ?2",
            params![file_id, write_id, version, object_version_id],
        )?;
        Ok(n == 1)
    }

    /// §8.1: every op waiting on `write_id` gets the version it produced.
    pub fn resolve_successors(&self, write_id: &str, version: i64, object_version_id: Option<&str>) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE operation_queue
             SET base_version = ?2, base_object_version_id = ?3, after_write_id = NULL
             WHERE after_write_id = ?1",
            params![write_id, version, object_version_id],
        )
    }

    #[cfg(test)]
    pub(crate) fn set_base_version_for_test(&self, op_id: &str, base_version: Option<i64>) {
        let conn = self.0.lock().unwrap();
        conn.execute("UPDATE operation_queue SET base_version = ?2 WHERE op_id = ?1", params![op_id, base_version]).unwrap();
    }
```

4.5 The claim's steps 3 and 4 (S3) and S5. Add `Parked { op_id: String, file_id: Option<String>, reason: ParkReason }` to `ClaimOutcome`. In `claim_operation`, after the content-order check and before setting the claim, read `let write = finder_write_conn(&tx, op_id)?;` and:

```rust
        if let Some(write) = &write {
            if write.base_pending > 0 {
                return Ok(ClaimOutcome::Wait); // step 3, not an attempt
            }
            if let Some(predecessor) = &write.after_write_id {
                let state: Option<(i64, i64)> = tx
                    .query_row(
                        "SELECT attempts, max_attempts FROM operation_queue WHERE write_id = ?1",
                        params![predecessor],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let reason = match state {
                    Some((attempts, max)) if attempts < max => return Ok(ClaimOutcome::Wait),
                    // Task 5 replaces this arm with the hand-over for a minted predecessor.
                    Some(_) => ParkReason::PredecessorParked,
                    // S5: only a bug can orphan a successor; it shows as a parked file.
                    None => ParkReason::PredecessorLost,
                };
                tx.execute(
                    "UPDATE operation_queue
                     SET attempts = max_attempts, last_error = ?2, last_error_class = ?2, updated_at = ?3
                     WHERE op_id = ?1",
                    params![op_id, reason.as_str(), now],
                )?;
                tx.commit()?;
                return Ok(ClaimOutcome::Parked { op_id: op_id.to_string(), file_id: op.file_id.clone(), reason });
            }
        }
```

(The later `let write = finder_write_conn(&tx, op_id)?;` before the commit reuses this value.)

- [ ] **Step 5: The engine side**

5.1 `FpWrite`, the error type and the log line:

```rust
/// A File Provider write's outcome, with the token its reply names (spec §5).
#[derive(Debug, Clone, PartialEq)]
pub struct FpWrite {
    pub outcome: FinderWriteOutcome,
    pub token: Option<String>,
}

impl FpWrite {
    pub fn plain(outcome: FinderWriteOutcome) -> Self {
        Self { outcome, token: None }
    }
}

/// Park the claimed op now, with its bytes kept (spec §8.4, §6.3.1).
#[derive(Debug)]
pub(crate) struct ParkNow(pub ParkReason);

impl std::fmt::Display for ParkNow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload parked: {}", self.0.as_str())
    }
}

impl std::error::Error for ParkNow {}

fn log_parked(op_id: &str, file_id: Option<&str>, reason: ParkReason) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        reason = reason.as_str(),
        "upload parked with its bytes kept in the queue"
    );
}
```

5.2 `queue_file_provider_create_from(target, contents)`:
- A folder (`FinderWriteItemKind::Folder`) carries no content: `return self.queue_finder_create_from(target, contents).map(FpWrite::plain);`.
- Otherwise repeat `queue_finder_create_from`'s file branch (`EB:1500-1531`, `EB:1577-1627`): the stop check, the ignored-name check, `ensure_shared_parent_allows_write`, a fresh `file_id`, `rel_path`, `stage_finder_contents`, size, MIME, `name_encrypted`, the `FileEntry` (`EB:1588-1601`) and the `create_file` metadata with `apply_shared_context`. Then, instead of `upsert_file` + `enqueue_finder_operation`:

```rust
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&rel_path);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                op_id: &op_id,
                file_id: &file_id,
                kind: crate::state_db::FinderAcceptKind::Create { row: &row },
                parent_id: target.parent_id.as_deref(),
                target_path: Some(&rel_path),
                metadata_json: &metadata_json,
                payload_path: &staged_path,
                size_bytes,
                modified_at: row.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        staged.retain();
        let token = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => token,
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&file_id), reason);
                token
            }
            crate::state_db::AcceptOutcome::UnknownItem => anyhow::bail!("a create always inserts its row"),
        };
        Ok(FpWrite {
            outcome: FinderWriteOutcome::Queued {
                op_id,
                file_id: Some(file_id),
                kind: OperationKind::UploadVersion,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            },
            token: Some(token),
        })
```

5.3 `queue_file_provider_modify_from(target, contents)`: without `contents_path` (a rename or move) it is `self.queue_finder_modify_from(target, contents).map(FpWrite::plain)`. With contents, repeat `EB:1641-1694` up to the payload, with no `get_file` / `modify_base_version` / `record_local_write` calls (the accept does those inside its transaction), then `accept_finder_write` with `FinderAcceptKind::Modify { incoming_base: target.base_version_identifier.as_deref() }`, `target_path: Some(&target.filename)` and `parent_id: target.parent_id.as_deref()`. `UnknownItem` keeps today's behaviour until Task 8 refuses it: enqueue the already-staged copy with `self.enqueue_finder_operation(OperationKind::UploadVersion, Some(file_id), target.parent_id, Some(target.filename), payload, Some(staged_path), parse_base_version_number(target.base_version_identifier.as_deref()), None)?`, `staged.retain()`, `Ok(FpWrite::plain(outcome))`. (The opened `contents` file was already read by the staging copy; it is never read twice.)

5.4 The `init` guard (§6.3.1). `upload_init_request_for_operation` (`EB:3640-3662`) gains `has_write_id: bool` and returns `anyhow::Result<DesktopUploadInitRequest>`:

```rust
    let base_version_number = if has_write_id {
        match (is_new_file, base_version) {
            (true, _) => None,
            (false, None) => return Err(anyhow::Error::new(ParkNow(ParkReason::BaseUnknown))),
            // A base that does not fit the server's i32 is never sent as "no base".
            (false, Some(version)) => Some(
                i32::try_from(version).map_err(|_| anyhow::Error::new(ParkNow(ParkReason::BaseUnknown)))?,
            ),
        }
    } else {
        // Windows and watcher uploads: unchanged.
        base_version.and_then(|version| i32::try_from(version).ok())
    };
```

The call site (`EB:767-775`) passes `claim.and_then(|claim| claim.write.as_ref()).is_some()` and `?`s the result.

5.5 The runner parks on `ParkNow` and on `ClaimOutcome::Parked`. In `process_due_operations`, add before `Err(error) =>`:

```rust
                Err(error) if error.downcast_ref::<ParkNow>().is_some() => {
                    let reason = error.downcast_ref::<ParkNow>().map(|park| park.0).unwrap_or(ParkReason::BaseUnknown);
                    if self.db.park_claimed(&op.op_id, &claimed.claim_id, reason, now)? {
                        log_parked(&op.op_id, op.file_id.as_deref(), reason);
                    } else {
                        log_queue_state_moved(&op.op_id, "park");
                    }
                    outcome.retried_op_ids.push(op.op_id);
                }
```

and the claim match gains `ClaimOutcome::Parked { op_id, file_id, reason } => { log_parked(&op_id, file_id.as_deref(), reason); outcome.retried_op_ids.push(op_id); continue; }`.

5.6 The landing keeps the held columns and resolves successors by write id. In `do_upload_version`'s `Ok(completed)` arm (`EB:815`), first `self.seam("landing:after_complete");`. In `apply_completed_upload`, before `self.db.delete_file(local_file_id)?` (`EB:1190`), add `self.db.carry_held_write(local_file_id, server_file_id)?;`. After `self.chain_queued_ops_after_upload(...)` (`EB:828`):

```rust
                if let Some(write) = claim.and_then(|claim| claim.write.as_ref())
                    && let Some(contract) = self.db.get_file_contract_state(&server_file_id)?
                {
                    let object = contract.current_object_version_id.as_deref();
                    self.db.set_held_landed(&server_file_id, &write.write_id, contract.current_version, object)?;
                    self.db.resolve_successors(&write.write_id, contract.current_version, object)?;
                }
```

`chain_queued_ops_after_upload` (`EB:1216-1258`) keeps the re-key for every op, and keeps its equal-base rebase only when neither the landed op nor the moved op is a Finder write (`claim` carried no `write`, and `self.db.finder_write(&op.op_id)?.is_none()`). Finder writes move by write id only.

5.7 `ipc_socket.rs`: on macOS the create arm calls `work_bridge.queue_file_provider_create_from(target, opened)` (`IPC:1832`) and the modify arm `work_bridge.queue_file_provider_modify_from(target, opened)` (`IPC:1877`); on other unix builds they keep `queue_finder_create_from` / `queue_finder_modify_from`, wrapped with `.map(FpWrite::plain)` (plan Spec issue 13). Use `#[cfg(target_os = "macos")]` / `#[cfg(not(target_os = "macos"))]` on two `let result = …;` statements. `write_outcome_response` takes `anyhow::Result<crate::engine_bridge::FpWrite>` and matches on `.outcome`; the delete arm passes `bridge.queue_finder_delete(&file_id, base_version_identifier).map(crate::engine_bridge::FpWrite::plain)`.

- [ ] **Step 6: GREEN, mutations**

Run the step-2 command into `$EVID/r4-t3-green.log`. Expected `ok. 8 passed`, and every test in the versioned section green (T10). Mutations:
- T12: rule 6 returns `Resolved { base: facts.current_version }` regardless of `version_filled` → "nothing is sent" fails (`$EVID/r4-t3-mut-T12.log`).
- T14: drop the `has_write_id` branch of the guard → "no replace without a base" fails (`…-T14.log`).
- T21: in `accept_finder_write`, when `decision` is `After { write_id, .. }` and no `upload_resume` row exists for that write's op, `UPDATE` that op's `payload_path` to this write's copy and skip the `INSERT` (the old supersede) → "one upload per save, in order" fails (`…-T21.log`).
- T22: in `decide_base`, rule 1a returns `Resolved { base: held.base }` → the second init carries base 1 and gets 409 (`…-T22.log`).
- T39: compute the decision from `self.db.get_file` / `get_file_contract_state` / a separate facts read before `self.seam("accept:before_tx")` and pass it into the accept → N waits on a write that is gone, parks `predecessor_lost`, and "N is based on W's produced version" fails (`…-T39.log`).
- T49: rule 3 picks the newest write whose base is resolved instead of `held` (the old rule 3 text, m-1) → the third save gets 409 (`…-T49.log`).
- T50: rule 5 treats any token (`parse_token(..).is_some()`) as no base → the guard parks it and the init summary is empty (`…-T50.log`).
- P5: store `None` as `base_version` for `After` → "after W1, stored as W1's b" fails (`…-P5.log`).

- [ ] **Step 7: Gate and commit**

Task gate; expected lib count **`L0 + 15`**. The IPC framing tests stay green with replies still reporting `"1"` (`ipc_socket_framing_tests.rs:900-905`).

```bash
cd $WT && git commit -m "feat(desktop): accept File Provider writes in one transaction and map their base through the write token" -- src-tauri/src/write_token.rs src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket.rs
```

**Stop point:** after the commit. Report counts, the eight mutation logs, and the list of round-3 tests whose entry point changed in step 1.1.

---

## Task 4: Rule 1: every surface reports the token (spec §5.3–§5.5, §5.4 rows 5–15, §8.5 m-9, §10.5 m-12; commit 4)

From here the reply, enumeration, the change feed and item lookups name the accepted bytes with the token, and keep it through our own landing.

**Files:**
- Modify: `src-tauri/src/ipc_socket.rs`: `file_entry_payload_for_db` (`IPC:2573-2600`); a test-only builder seam
- Modify: `src-tauri/src/state_db.rs`: `clear_held_if`, `apply_restore_response`; `purge_all_local_state` (`SD:2874-2944`); test-only `insert_alias_for_test`
- Modify: `src-tauri/src/engine_bridge.rs`: the `RestoreVersion` arm (`EB:632-643`); the mock gains the restore route
- Modify: `src-tauri/src/ipc_socket_framing_tests.rs`: `a_queued_modify_replies_with_the_size_of_the_bytes_it_was_handed` (`:853-905`) asserts the token
- Test: `engine_bridge.rs` tests

**Interfaces:**
- Consumes: `item_presentation`, `held_token` (Task 1); `fp_save`, `fp_create`, `FpWrite` (Task 3); `run_competing` (Task 2).
- Produces:
  - `StateDb::clear_held_if(&self, file_id: &str, write_id: &str) -> Result<bool>` (§5.3: `WHERE file_id = ? AND held_write_id = ?`)
  - `StateDb::apply_restore_response(&self, file_id: &str, version: Option<i64>, object_version_id: Option<&str>) -> Result<()>` (§5.4 row 10, one transaction; plan Spec issue 5)
  - `#[cfg(test)] StateDb::insert_alias_for_test(&self, provisional_id: &str, server_id: &str, created_at: i64)`
  - `#[cfg(test)] ipc_socket::arm_builder_seam(hook: impl FnOnce() + 'static)`: fires once, on the calling thread, right after `file_entry_payload_for_db`'s one read
  - mock: `POST /api/v1/files/{id}/versions/{vid}/restore` appends a copy of version `vid` (the mock's object ids are `object-{id}-v{n}`) and answers `{"message": "version restored", "version_number": n, "current_object_version_id": "object-{id}-v{n}"}` (the server's first branch, `VER:613-618`); `VersionedServerState` gains `restores: Vec<(String, String)>`

- [ ] **Step 1: Write the failing tests**

Change the framing assertion at `ipc_socket_framing_tests.rs:900-905` to:

```rust
    let content_version = item["content_version"].as_str().unwrap();
    assert!(
        crate::write_token::parse_token(content_version).is_some_and(|token| token.base == 1),
        "the reply names the bytes it accepted with a token led by their base: {reply}"
    );
```

Add to `engine_bridge.rs` tests (setup as in Task 3; the master-key byte is given in each):

```rust
    #[tokio::test]
    async fn a_landing_keeps_the_content_version_the_reply_named() {
        // master_key [52u8; 32]
        seed_uploaded_row(&bridge, &server, "named");
        let reply = fp_save(&bridge, dir.path(), "named", "notes.txt", b"edit", "1");
        let token = reply.token.clone().unwrap();
        assert!(token.starts_with("1:w"), "{token}");
        assert_eq!(held_content_version(&bridge, "named"), token, "while queued");
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(held_content_version(&bridge, "named"), token, "after our own landing");
        drop(server.finish());
    }

    /// W landed as v2 on `file_id`; returns W's token. Used by T2, T3, T4, T5, T6, T64.
    async fn land_one_save(bridge: &EngineBridge, server: &VersionedServerMock, dir: &Path, sync_root: &Path, file_id: &str) -> String {
        seed_uploaded_row(bridge, server, file_id);
        let token = fp_save(bridge, dir, file_id, "notes.txt", b"landed edit", "1").token.unwrap();
        drain_upload_queue(bridge, sync_root).await;
        assert_eq!(held_content_version(bridge, file_id), token);
        token
    }

    fn remote_update(bridge: &EngineBridge, sync_root: &Path, file_id: &str, version: i64, object: &str) {
        let op = crate::api_client::SyncOp {
            seq_id: 100 + version,
            op_type: "file_update".into(),
            payload: serde_json::json!({ "id": file_id, "version_number": version, "current_object_version_id": object, "size_bytes": 11 }),
        };
        apply_sync_op(bridge, sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
    }

    #[tokio::test]
    async fn a_remote_change_replaces_the_token() {
        // master_key [53u8; 32]
        land_one_save(&bridge, &server, dir.path(), &sync_root, "remote").await;
        remote_update(&bridge, &sync_root, "remote", 3, "object-elsewhere-v3");
        assert_eq!(held_content_version(&bridge, "remote"), "3");
        assert!(bridge.db.item_presentation("remote").unwrap().unwrap().held.is_none(), "the builder cleared it");
        drop(server.finish());
    }

    #[tokio::test]
    async fn our_own_echo_keeps_the_token() {
        // master_key [54u8; 32]
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "echo").await;
        remote_update(&bridge, &sync_root, "echo", 2, "object-echo-v2");
        assert_eq!(held_content_version(&bridge, "echo"), token, "the echo of our own landing changes nothing");
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_restore_from_this_mac_replaces_the_token() {
        // master_key [55u8; 32]
        land_one_save(&bridge, &server, dir.path(), &sync_root, "restored").await;
        bridge.queue_restore_version("restored", "object-restored-v1", None).unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(held_content_version(&bridge, "restored"), "3", "the restore's version, so the system re-downloads");
        drop(server.finish());
    }

    #[tokio::test]
    async fn the_token_survives_rename_move_and_trash() {
        // master_key [56u8; 32]
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "moved").await;
        let rename = FinderWriteTarget {
            file_id: Some("moved".into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: Some(token.clone()),
        };
        bridge.queue_file_provider_modify(rename).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "rename");
        bridge.queue_finder_delete("moved", Some(token.clone())).unwrap();
        assert_eq!(held_content_version(&bridge, "moved"), token, "trash");
        drop(server.finish());
    }

    #[tokio::test]
    async fn sign_out_clears_tokens_and_aliases() {
        // master_key [57u8; 32]
        land_one_save(&bridge, &server, dir.path(), &sync_root, "signed-out").await;
        bridge.db.insert_alias_for_test("provisional-1", "signed-out", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(bridge.db.item_presentation("signed-out").unwrap().unwrap().held.is_none());
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[tokio::test]
    async fn sign_out_purges_provisional_rows_without_a_contract() {
        // master_key [58u8; 32]
        seed_uploaded_row(&bridge, &server, "server-known");
        let created = fp_create(&bridge, dir.path(), "never-uploaded.txt", b"local only");
        let provisional = created.outcome_file_id();
        bridge.db.insert_alias_for_test("older-provisional", "server-known", now_secs());
        bridge.db.purge_all_local_state().unwrap();
        assert!(bridge.db.get_file(&provisional).unwrap().is_none(), "no ghost row after sign-out");
        assert!(bridge.db.get_file("server-known").unwrap().is_some(), "server-known rows are kept");
        assert_eq!(bridge.db.alias_count_for_test(), 0);
        drop(server.finish());
    }

    #[tokio::test]
    async fn c1_a_save_after_the_first_landed_is_based_on_what_it_produced() {
        // master_key [59u8; 32]; the review's two passes
        seed_uploaded_row(&bridge, &server, "c1");
        fp_save(&bridge, dir.path(), "c1", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let base = held_content_version(&bridge, "c1"); // what the system holds after re-reading
        fp_save(&bridge, dir.path(), "c1", "notes.txt", b"A B", &base);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("c1"), json!(1), 201), (json!("c1"), json!(2), 201)]);
        assert_eq!(state.latest_plaintext("c1", master_key), b"A B");
    }

    #[tokio::test]
    async fn c1_three_saves_in_one_pass() {
        // master_key [60u8; 32]
        server.state.lock().unwrap().delay_chunks.insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "c3");
        let a = fp_save(&bridge, dir.path(), "c3", "notes.txt", b"A", "1");
        let ((), ()) = tokio::join!(
            async { drain_upload_queue(&bridge, &sync_root).await; },
            async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let b = fp_save(&bridge, dir.path(), "c3", "notes.txt", b"A B", a.token.as_deref().unwrap());
                fp_save(&bridge, dir.path(), "c3", "notes.txt", b"A B C", b.token.as_deref().unwrap());
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("c3"), json!(1), 201), (json!("c3"), json!(2), 201), (json!("c3"), json!(3), 201)]
        );
        assert_eq!(state.latest_plaintext("c3", master_key), b"A B C");
    }

    #[tokio::test]
    async fn a_restore_with_a_queued_write_runs_after_it_and_keeps_both_versions() {
        // master_key [61u8; 32]; m-9
        seed_uploaded_row(&bridge, &server, "ordered");
        let w = fp_save(&bridge, dir.path(), "ordered", "notes.txt", b"W", "1");
        bridge.queue_restore_version("ordered", "object-ordered-v1", None).unwrap();
        assert_eq!(held_content_version(&bridge, "ordered"), w.token.clone().unwrap(), "kept while W is queued");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files["ordered"].versions.len(), 3, "v1, W as v2, the restore as v3");
        let complete_at = state.requests.iter().position(|(m, p)| m == "POST" && p.ends_with("/complete")).unwrap();
        let restore_at = state.requests.iter().position(|(m, p)| m == "POST" && p.ends_with("/restore")).unwrap();
        assert!(complete_at < restore_at, "W lands first: {:?}", state.requests);
        assert_eq!(held_content_version(&bridge, "ordered"), "3");
    }

    #[tokio::test]
    async fn a_builder_clears_held_columns_only_for_the_write_it_read() {
        // master_key [62u8; 32]
        let bridge = Arc::new(bridge);
        land_one_save(&bridge, &server, dir.path(), &sync_root, "guarded").await;
        remote_update(&bridge, &sync_root, "guarded", 3, "object-elsewhere-v3");
        // Between the builder's read (W, no longer current) and its clear, a save sets held = N.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = Arc::new(Mutex::new(None));
        let n_slot = Arc::clone(&n_token);
        crate::ipc_socket::arm_builder_seam(move || {
            let n = fp_save(&saver, &save_dir, "guarded", "notes.txt", b"N", "3");
            *n_slot.lock().unwrap() = n.token;
        });
        let _ = held_content_version(&bridge, "guarded");
        let n = n_token.lock().unwrap().clone().unwrap();
        let held = bridge.db.item_presentation("guarded").unwrap().unwrap().held.unwrap();
        assert_eq!(held.token(), n, "the clear matched no row; N's token is kept");
        drop(server.finish());
    }

    #[tokio::test]
    async fn the_predicate_reads_row_and_queue_together() {
        // master_key [63u8; 32]
        let bridge = Arc::new(bridge);
        seed_uploaded_row(&bridge, &server, "one-read");
        let w = fp_save(&bridge, dir.path(), "one-read", "notes.txt", b"W", "1").token.unwrap();
        let runner = Arc::clone(&bridge);
        let root = sync_root.clone();
        crate::ipc_socket::arm_builder_seam(move || {
            run_competing(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async { runner.process_due_operations(&root, now_secs()).await.unwrap(); });
            });
        });
        assert_eq!(held_content_version(&bridge, "one-read"), w, "never the old {{cv}} while W lands");
        drop(server.finish());
    }

    #[tokio::test]
    async fn twenty_rapid_saves_land_in_order_and_the_last_wins() {
        // master_key [64u8; 32]; Review Focus 1
        server.state.lock().unwrap().delay_chunks.insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "autosave");
        let first = fp_save(&bridge, dir.path(), "autosave", "notes.txt", b"save 1", "1");
        let mut base = first.token.unwrap();
        let mut last = b"save 1".to_vec();
        let ((), ()) = tokio::join!(
            async { drain_upload_queue(&bridge, &sync_root).await; },
            async {
                while server.state.lock().unwrap().inits.is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                for n in 2..=20 {
                    let bytes = format!("save {n}").into_bytes();
                    let reply = fp_save(&bridge, dir.path(), "autosave", "notes.txt", &bytes, &base);
                    base = reply.token.unwrap();
                    assert_eq!(held_content_version(&bridge, "autosave"), base, "save {n}: one name for its bytes");
                    last = bytes;
                }
            }
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        let bases: Vec<i64> = state
            .inits
            .iter()
            .map(|(body, status)| {
                assert_eq!(*status, 201, "{:?}", state.init_summary());
                body["base_version_number"].as_i64().unwrap()
            })
            .collect();
        assert_eq!(bases, (1..=20).collect::<Vec<i64>>(), "one upload per save, in order");
        assert_eq!(state.latest_plaintext("autosave", master_key), last, "the last save wins");
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty());
        assert_eq!(held_content_version(&bridge, "autosave"), base, "the last save's name stays after it lands");
    }
```

(`alias_count_for_test(&self) -> i64` is a test-only `SELECT COUNT(*) FROM id_aliases`. In T64 and T65, `bridge` is wrapped in `Arc` before use; `EngineBridge` methods are called through the `Arc`.)

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- a_landing_keeps_the_content_version a_remote_change_replaces our_own_echo_keeps a_restore_from_this_mac the_token_survives sign_out_clears_tokens sign_out_purges_provisional c1_a_save_after c1_three_saves a_restore_with_a_queued_write a_builder_clears_held the_predicate_reads_row twenty_rapid_saves a_queued_modify_replies_with_the_size > $EVID/r4-t4-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t4-red.log | head -30
```

Expected REDs (after the test-only helpers compile): T1 `"1"` ≠ the token; T3 and T5 the same; T8 the second save on `"1"` gets `(c1,1,409)`; T9 `(c3,1,409)`; T4 `"1"` (the response is discarded, `EB:632-642`); T57 the restore is not reported (`"1"`); T6, T62 held columns, aliases and the provisional row survive the purge; T64, T65 `arm_builder_seam` missing; RF1 409s from save 2 on; the framing test gets `"1"`. T2 is a guard (spec T2: "green today").

- [ ] **Step 3: Implement**

3.1 `state_db.rs`:

```rust
    /// §5.3: housekeeping only, guarded by the write the caller read.
    pub fn clear_held_if(&self, file_id: &str, write_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE file_id = ?1 AND held_write_id = ?2",
            params![file_id, write_id],
        )?;
        Ok(n == 1)
    }

    /// §5.4 row 10 (m-9): the restore's version becomes current; the held columns are
    /// cleared only when no Finder write of the file is queued (plan Spec issue 5).
    pub fn apply_restore_response(&self, file_id: &str, version: Option<i64>, object_version_id: Option<&str>) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(version) = version {
            tx.execute(
                "UPDATE files SET current_version = ?2, local_base_version = ?2,
                                  current_object_version_id = ?3, version_filled = 0
                 WHERE file_id = ?1",
                params![file_id, version, object_version_id],
            )?;
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        }
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE file_id = ?1
               AND NOT EXISTS (SELECT 1 FROM operation_queue
                               WHERE file_id = ?1 AND write_id IS NOT NULL
                                 AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
        )?;
        tx.commit()
    }
```

In `purge_all_local_state` (`SD:2874`), right after `let tx = conn.transaction()?;` collect the provisional rows, and after `DELETE FROM operation_queue` remove them and the round-4 state:

```rust
        // m-12: provisional rows (a create that never landed) would survive as ghosts.
        let provisional: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT q.file_id FROM operation_queue q JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind IN ('upload_version', 'upload_file')
                   AND json_extract(q.metadata_json, '$.operation') = 'create_file'
                   AND f.current_version = 0",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        // … the existing `DELETE FROM operation_queue` (SD:2882) …
        for file_id in &provisional {
            record_file_change_conn(&tx, file_id, FpChangeKind::Deleted, None)?;
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
        }
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                              held_object_version_id = NULL
             WHERE held_write_id IS NOT NULL",
            [],
        )?;
        tx.execute("DELETE FROM id_aliases", [])?;
```

3.2 `ipc_socket.rs`. The test-only seam:

```rust
#[cfg(test)]
thread_local! {
    static BUILDER_SEAM: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// T64/T65: fires once, on this thread, right after the builder's one read.
#[cfg(test)]
pub(crate) fn arm_builder_seam(hook: impl FnOnce() + 'static) {
    BUILDER_SEAM.with(|seam| *seam.borrow_mut() = Some(Box::new(hook)));
}

fn builder_seam() {
    #[cfg(test)]
    {
        let hook = BUILDER_SEAM.with(|seam| seam.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }
}
```

`file_entry_payload_for_db` (`IPC:2573-2600`) becomes:

```rust
pub(crate) fn file_entry_payload_for_db(
    db: &crate::state_db::StateDb,
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    // §5.3: the row and the queue come from ONE read.
    let presentation = db.item_presentation(&entry.file_id).ok().flatten();
    builder_seam();
    let mut payload = match &presentation {
        Some(p) => file_entry_payload(entry, &p.contract, parent_identifier),
        None => file_entry_payload_without_contract(entry, parent_identifier),
    };
    if let Some(p) = &presentation {
        match crate::write_token::held_token(
            p.held.as_ref(),
            p.held_write_queued,
            p.contract.current_version,
            p.contract.current_object_version_id.as_deref(),
        ) {
            Some(token) => payload.content_version = Some(token),
            None => {
                if let Some(held) = &p.held {
                    let _ = db.clear_held_if(&entry.file_id, &held.write_id);
                }
            }
        }
    }
    // … the rest of today's body from `payload.modified_at = …` (IPC:2585) on, unchanged …
}
```

`version_identifier` keeps its round-3 meaning (§5.3).

3.3 `engine_bridge.rs` `RestoreVersion` arm (`EB:632-643`): keep the response and apply it:

```rust
                let response = self.api.restore_version(file_id, version_id).await?;
                self.db.apply_restore_response(
                    file_id,
                    response["version_number"].as_i64(),
                    response["current_object_version_id"].as_str(),
                )?;
                Ok(None)
```

3.4 The mock's restore route, in `versioned_server_answer` before the `PATCH` branch:

```rust
        if method == "POST"
            && let Some(rest) = path.strip_prefix("/api/v1/files/")
            && rest.ends_with("/restore")
        {
            let parts: Vec<&str> = rest.split('/').collect(); // [id, "versions", vid, "restore"]
            let (file_id, object) = (parts[0].to_string(), parts[2].to_string());
            let index = object.rsplit("-v").next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1);
            let file = s.files.entry(file_id.clone()).or_default();
            let chunks = file.versions.get(index).cloned().unwrap_or_default();
            file.versions.push(chunks);
            let version = file.versions.len();
            s.restores.push((file_id.clone(), object));
            return http_json(
                "200 OK",
                serde_json::json!({
                    "message": "version restored",
                    "version_number": version,
                    "current_object_version_id": format!("object-{file_id}-v{version}"),
                }),
            );
        }
```

- [ ] **Step 4: GREEN, mutations**

Step-2 command into `$EVID/r4-t4-green.log`; expected `ok. 14 passed` (13 new and the changed framing test). Mutations (one log each, `$EVID/r4-t4-mut-<T>.log`):
- T1: `file_entry_payload_for_db` reports the token only while `held_write_queued` → "after our own landing" fails.
- T2: `held_token` ignores `current_version` when the object ids match → `"3"` assertion fails.
- T3: clear the held columns on every applied op (`apply_sync_op`) → the token is lost.
- T4: drop the `apply_restore_response` call → `"1"`.
- T5: clear the held columns in `queue_finder_delete` → "trash" fails.
- T6: drop the held-columns `UPDATE` from the purge → held is still set.
- T8: rule 1b returns `Resolved { base: held.base }` → `(c1,1,409)`.
- T9: replace `resolve_successors` with the round-3 equal-base rebase for Finder writes → C waits on a write that is gone and parks `predecessor_lost`; the init summary has two entries.
- T57: drop `'restore_version'` from the claim's content order (Task 2) → the restore lands before W.
- T62: drop the provisional-row delete → the ghost row survives.
- T64: `clear_held_if` without `AND held_write_id = ?2` → N's token is cleared.
- T65: split `item_presentation` into two locked calls (held columns and contract first, then the queue facts), and move `builder_seam()` between them → the payload reports `"1"`.
- RF1: rule 1a returns `Resolved { base: held.base }` → 409 from save 2 on.

- [ ] **Step 5: Gate and commit**

Task gate; expected lib count **`L0 + 28`**.

```bash
cd $WT && git commit -m "feat(desktop): every File Provider surface reports the write token until a remote change" -- src-tauri/src/ipc_socket.rs src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket_framing_tests.rs
```

**Stop point:** after the commit.

---

## Task 5: Rule 4: 409 classes, the immediate parks, the hand-over at the claim (spec §8.4; commit 5)

A doomed earlier write no longer blocks the newest save for hours (I4): a stale base or a missing payload parks at once, and the next claim of its successor takes over.

**Files:**
- Modify: `src-tauri/src/api_client.rs`: `init_upload` (`api_client.rs:731-741`); new `InitConflictClass`, `InitConflict`, `classify_init_conflict`, the two message constants
- Modify: `src-tauri/src/engine_bridge.rs`: `error_http_status` (`EB:3992-3999`); `log_refused_upload` (`EB:4004-4027`) gains `class` and, when parking, `reason`; the payload check (`EB:709-713`); `process_due_operations` handles stale-base parks and the claim's hand-over outcomes; the mock gains `conflict_init_once`
- Modify: `src-tauri/src/state_db.rs`: the claim's step 4 hands over (`TookOver`, `ClaimOutcome::WaitAfterHandOver`, `ClaimedOp.took_over`); test-only `set_write_origin_for_test`, `park_for_test`
- Test: `engine_bridge.rs` tests; the existing `an_upload_refused_with_409_is_logged_on_every_retry_and_when_it_parks` (`EB:13386-13462`) is rewritten in place

**Interfaces:**
- Consumes: the claim, `park_claimed`, `ParkNow`, `log_parked` (Tasks 2–3); `fp_save`, `fp_create` (Task 3).
- Produces:
  - `api_client::{STALE_BASE_MESSAGE: &str = "stale base version for replacement upload", IN_PROGRESS_MESSAGE: &str = "upload is already in progress for this file"}` (the server's messages: `UP:777`, `UP:732`, `UP:773` at `46e5054c`; `uploads.rs:861`, `:816`, `:857` at `62157c52`)
  - `api_client::InitConflictClass { StaleBase, InProgress, Other }`, `as_str()` → `"stale_base"`, `"in_progress"`, `"other"`; `api_client::classify_init_conflict(&str) -> InitConflictClass`; `api_client::InitConflict { pub class: InitConflictClass }` (an error type; `Display`: `upload init refused with 409 Conflict (<class>)`, never the server's text)
  - `engine_bridge::init_conflict_class(&anyhow::Error) -> Option<InitConflictClass>`
  - `state_db::TookOver { pub parked_op_id: String, pub released_payload: Option<String> }`; `ClaimedOp.took_over: Option<TookOver>`; `ClaimOutcome::WaitAfterHandOver(TookOver)` (the hand-over committed, and the successor itself must still wait)
  - `fn log_took_over(op_id: &str, file_id: Option<&str>, parked_op_id: &str)` → `warn!` "queued write took over a parked one" with `op_id`, `file_id`, `parked_op_id`
  - mock: `VersionedServerState.conflict_init_once: HashMap<String, String>` (file id → the 409 message its next `init` gets)

- [ ] **Step 1: Write the failing tests**

Rewrite the existing 409 test in place as `an_upload_refused_with_a_stale_base_is_logged_once_and_parks`: drop the `stale.max_attempts = 3` re-enqueue; assert one `409` init for `stale-file`; exactly one `upload refused` line, which contains `parked`, `stale_base`, the op id and `stale-file`; no line for the flaky op; no `notes.txt`, `/api/v1` or `127.0.0.1` in any line. Then add (setup as in Task 3):

```rust
    #[test]
    fn the_409_messages_are_pinned() {
        use crate::api_client::{InitConflictClass, classify_init_conflict};
        assert_eq!(classify_init_conflict("stale base version for replacement upload"), InitConflictClass::StaleBase);
        assert_eq!(classify_init_conflict("upload is already in progress for this file"), InitConflictClass::InProgress);
        assert_eq!(classify_init_conflict("file is in trash"), InitConflictClass::Other);
        assert_eq!(classify_init_conflict(""), InitConflictClass::Other, "a changed message falls back to retry");
    }

    #[tokio::test]
    async fn a_stale_base_409_parks_at_once() {
        // master_key [65u8; 32]
        seed_uploaded_row(&bridge, &server, "stale");
        server.seed_file("stale", 2); // the server moved on
        fp_save(&bridge, dir.path(), "stale", "notes.txt", b"edit on v1", "1");
        let logs = capture_logs_async(async {
            bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        })
        .await;
        let op = bridge.db.list_operations_for_file("stale").unwrap().remove(0);
        assert_eq!(op.attempts, op.max_attempts, "parked after one attempt");
        let refused: Vec<&str> = logs.lines().filter(|l| l.contains("upload refused")).collect();
        assert_eq!(refused.len(), 1, "{logs}");
        assert!(refused[0].contains("stale_base") && refused[0].contains("parked"), "{logs}");
        drop(server.finish());
    }

    #[tokio::test]
    async fn an_in_progress_409_is_retried() {
        // master_key [66u8; 32]
        seed_uploaded_row(&bridge, &server, "busy");
        server.state.lock().unwrap().conflict_init_once.insert("busy".into(), crate::api_client::IN_PROGRESS_MESSAGE.into());
        fp_save(&bridge, dir.path(), "busy", "notes.txt", b"edit", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("busy"), json!(1), 409), (json!("busy"), json!(1), 201)]);
        assert_eq!(state.latest_plaintext("busy", master_key), b"edit");
    }

    #[tokio::test]
    async fn i4_a_doomed_earlier_write_never_blocks_hours() {
        // master_key [67u8; 32]
        seed_uploaded_row(&bridge, &server, "doomed");
        let w = fp_save(&bridge, dir.path(), "doomed", "notes.txt", b"W", "1");
        fp_save(&bridge, dir.path(), "doomed", "notes.txt", b"W N", w.token.as_deref().unwrap());
        let w_op = bridge.db.list_operations_for_file("doomed").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        let (passes, logs) = {
            let mut passes = 0;
            let logs = capture_logs_async(async { passes = drain_upload_queue(&bridge, &sync_root).await; }).await;
            (passes, logs)
        };
        let state = server.finish();
        assert!(passes <= 2, "the successor lands in the same or the next pass: {passes}");
        assert_eq!(state.init_summary(), vec![(json!("doomed"), json!(1), 201)], "N took W's base");
        assert_eq!(state.latest_plaintext("doomed", master_key), b"W N");
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
        assert!(bridge.db.list_operations_for_file("doomed").unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_base_predecessor_hands_over_and_the_successor_parks_with_the_newest_bytes() {
        // master_key [68u8; 32]
        seed_uploaded_row(&bridge, &server, "stale-chain");
        server.seed_file("stale-chain", 2);
        let w = fp_save(&bridge, dir.path(), "stale-chain", "notes.txt", b"W", "1");
        fp_save(&bridge, dir.path(), "stale-chain", "notes.txt", b"W N", w.token.as_deref().unwrap());
        let logs = capture_logs_async(async { drain_upload_queue(&bridge, &sync_root).await; }).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("stale-chain"), json!(1), 409), (json!("stale-chain"), json!(1), 409)],
            "W and then N, on W's base"
        );
        let ops = bridge.db.list_operations_for_file("stale-chain").unwrap();
        assert_eq!(ops.len(), 1, "W's op is gone; one parked op remains");
        assert_eq!(ops[0].attempts, ops[0].max_attempts);
        assert_eq!(std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(), b"W N", "the newest bytes are kept");
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn an_earlier_builds_op_is_never_handed_over() {
        // master_key [69u8; 32]; m-2
        // (a) an earlier-build op and a newer save on "1": both sets of bytes kept.
        seed_uploaded_row(&bridge, &server, "earlier");
        fp_save(&bridge, dir.path(), "earlier", "notes.txt", b"earlier build's bytes", "1");
        let e = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        bridge.db.set_write_origin_for_test(&e.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        fp_save(&bridge, dir.path(), "earlier", "notes.txt", b"a newer save", "1");
        // (b) a provisional row whose earlier-build create parks with a save queued after it.
        let created = fp_create(&bridge, dir.path(), "made-earlier.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(&bridge, dir.path(), &provisional, "made-earlier.txt", b"created, edited", created.token.as_deref().unwrap());
        let c = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        bridge.db.set_write_origin_for_test(&c.op_id, crate::state_db::WriteOrigin::EarlierBuild);
        bridge.db.park_for_test(&c.op_id);
        let logs = capture_logs_async(async { drain_upload_queue(&bridge, &sync_root).await; }).await;
        let state = server.finish();
        assert_eq!(state.latest_plaintext("earlier", master_key), b"earlier build's bytes", "(a) E landed");
        let newer = bridge.db.list_operations_for_file("earlier").unwrap().remove(0);
        assert_eq!(newer.attempts, newer.max_attempts, "(a) the newer save parked, not dropped");
        assert!(std::path::Path::new(newer.payload_path.as_deref().unwrap()).is_file());
        let ops = bridge.db.list_operations_for_file(&provisional).unwrap();
        assert_eq!(ops.len(), 2, "(b) no hand-over: C and N both remain");
        assert!(ops.iter().all(|op| op.attempts == op.max_attempts));
        assert!(logs.contains("predecessor_parked"), "{logs}");
        assert!(!logs.contains("took over"), "{logs}");
        assert!(state.inits.iter().all(|(body, _)| body["file_id"] != json!(provisional)), "(b) nothing uploaded");
    }

    #[tokio::test]
    async fn handover_vs_new_save_the_newest_bytes_land_last() {
        // master_key [70u8; 32]
        let bridge = Arc::new(bridge);
        seed_uploaded_row(&bridge, &server, "handover");
        let w = fp_save(&bridge, dir.path(), "handover", "notes.txt", b"W", "1");
        let w_op = bridge.db.list_operations_for_file("handover").unwrap().remove(0);
        std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // W parks payload_missing
        let n = fp_save(&bridge, dir.path(), "handover", "notes.txt", b"W N", w.token.as_deref().unwrap());
        // At N's claim, a newer save N2 is accepted on N's token.
        let saver = Arc::clone(&bridge);
        let save_dir = dir.path().to_path_buf();
        let n_token = n.token.clone().unwrap();
        bridge.seams.arm("claim:before_tx", move || {
            run_competing(move || {
                fp_save(&saver, &save_dir, "handover", "notes.txt", b"W N N2", &n_token);
            });
        });
        let logs = capture_logs_async(async { drain_upload_queue(&bridge, &sync_root).await; }).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("handover"), json!(1), 201), (json!("handover"), json!(2), 201)],
            "N on W's base, then N2"
        );
        assert_eq!(state.latest_plaintext("handover", master_key), b"W N N2", "the newest bytes land last");
        assert_eq!(logs.matches("queued write took over a parked one").count(), 1, "{logs}");
    }

    #[tokio::test]
    async fn a_lost_reply_base_parks_with_its_bytes() {
        // master_key [71u8; 32]; Review Focus 4: the shipped fallback of the split rule 1c
        seed_uploaded_row(&bridge, &server, "lost-reply");
        let a = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A", "1");
        drain_upload_queue(&bridge, &sync_root).await;
        let a_token = a.token.unwrap();
        let _b_reply_lost = fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B", &a_token);
        fp_save(&bridge, dir.path(), "lost-reply", "notes.txt", b"A B C", &a_token); // still A's token
        let logs = capture_logs_async(async { drain_upload_queue(&bridge, &sync_root).await; }).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("lost-reply"), json!(1), 201), (json!("lost-reply"), json!(2), 201), (json!("lost-reply"), json!(1), 409)]
        );
        assert_eq!(state.latest_plaintext("lost-reply", master_key), b"A B");
        let parked = bridge.db.list_operations_for_file("lost-reply").unwrap().remove(0);
        assert_eq!(parked.attempts, parked.max_attempts, "a visible park");
        assert_eq!(std::fs::read(parked.payload_path.as_deref().unwrap()).unwrap(), b"A B C", "its bytes are kept");
        assert!(logs.lines().any(|l| l.contains("upload refused") && l.contains("parked")), "{logs}");
    }
```

(`set_write_origin_for_test` and `park_for_test` are test-only `UPDATE`s of `write_origin` and of `attempts = max_attempts`; T23 reads `drain_upload_queue`'s pass count, which it already returns.)

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- the_409_messages_are_pinned a_stale_base_409_parks an_in_progress_409 i4_a_doomed a_stale_base_predecessor an_earlier_builds_op handover_vs_new_save a_lost_reply_base an_upload_refused_with_a_stale_base > $EVID/r4-t5-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t5-red.log | head -20
```

Expected REDs: T24 and the rewritten test retry (attempts 1 of 25); T23 W retries its missing payload and N parks `predecessor_parked` (Task 3's arm); T67 likewise; T41 N parks; RF4 retries C instead of parking; P8, T25 do not compile until the `api_client` items exist (T25 is a guard). T26 is green on (b) already (Task 3 parks `predecessor_parked`); its RED comes from its mutation.

- [ ] **Step 3: The 409 classes (`api_client.rs`)**

```rust
/// The server's 409 messages at `init` (spec §8.4); a test pins both.
pub const STALE_BASE_MESSAGE: &str = "stale base version for replacement upload";
pub const IN_PROGRESS_MESSAGE: &str = "upload is already in progress for this file";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitConflictClass {
    StaleBase,
    InProgress,
    Other,
}

impl InitConflictClass {
    pub fn as_str(self) -> &'static str {
        match self {
            InitConflictClass::StaleBase => "stale_base",
            InitConflictClass::InProgress => "in_progress",
            InitConflictClass::Other => "other",
        }
    }
}

/// A changed server message falls back to `Other` (retry), today's behaviour.
pub fn classify_init_conflict(message: &str) -> InitConflictClass {
    match message {
        STALE_BASE_MESSAGE => InitConflictClass::StaleBase,
        IN_PROGRESS_MESSAGE => InitConflictClass::InProgress,
        _ => InitConflictClass::Other,
    }
}

/// `init` answered 409. Carries the class only: the server's text never reaches a log.
#[derive(Debug)]
pub struct InitConflict {
    pub class: InitConflictClass,
}

impl std::fmt::Display for InitConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload init refused with 409 Conflict ({})", self.class.as_str())
    }
}

impl std::error::Error for InitConflict {}
```

`init_upload` (`api_client.rs:731-741`):

```rust
    pub async fn init_upload(&self, body: &DesktopUploadInitRequest) -> anyhow::Result<DesktopUploadInitResponse> {
        let resp = self
            .client
            .post(self.upload_init_url())
            .header("Authorization", format!("Bearer {}", self.token))
            .json(body)
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::CONFLICT {
            // `error_for_status` would drop the message the class is read from.
            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
            let message = body.get("error").and_then(|value| value.as_str()).unwrap_or_default();
            return Err(anyhow::Error::new(InitConflict { class: classify_init_conflict(message) }));
        }
        Ok(resp.error_for_status()?.json().await?)
    }
```

- [ ] **Step 4: The engine**

4.1 `error_http_status` (`EB:3992`) first checks `error.chain().any(|c| c.downcast_ref::<crate::api_client::InitConflict>().is_some())` → `Some(409)`. Add:

```rust
pub(crate) fn init_conflict_class(error: &anyhow::Error) -> Option<crate::api_client::InitConflictClass> {
    error.chain().find_map(|cause| cause.downcast_ref::<crate::api_client::InitConflict>()).map(|conflict| conflict.class)
}
```

4.2 `log_refused_upload(op, attempt, class: InitConflictClass)` adds `class = class.as_str()` to both lines, and `reason = "stale_base"` to the parked line when `class` is `StaleBase`.

4.3 The payload check (`EB:709-713`) becomes:

```rust
        match std::fs::metadata(payload_path) {
            // The staged copy lives in the app's own container and cannot come back (§8.4).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::Error::new(ParkNow(ParkReason::PayloadMissing)));
            }
            Err(e) => return Err(anyhow::anyhow!("staged upload payload could not be read: {e}")),
            Ok(meta) if !meta.is_file() => return Err(anyhow::anyhow!("staged upload payload is not a file")),
            Ok(_) => {}
        }
```

4.4 In `process_due_operations`, a stale-base 409 parks at once. Before the generic `Err(error) =>` arm:

```rust
                Err(error) if init_conflict_class(&error) == Some(crate::api_client::InitConflictClass::StaleBase) => {
                    // A stale base never becomes valid again: the server's version only grows (UP:782-787).
                    if self.db.park_claimed(&op.op_id, &claimed.claim_id, ParkReason::StaleBase, now)? {
                        log_refused_upload(&op, op.max_attempts, crate::api_client::InitConflictClass::StaleBase);
                    } else {
                        log_queue_state_moved(&op.op_id, "park");
                    }
                    outcome.retried_op_ids.push(op.op_id);
                }
```

In the generic arm, the existing 409 log becomes `log_refused_upload(&op, attempts, init_conflict_class(&error).unwrap_or(crate::api_client::InitConflictClass::Other))`.

4.5 The claim outcomes. The claim match becomes:

```rust
            let claimed = match self.db.claim_operation(&op.op_id, now)? {
                ClaimOutcome::Gone | ClaimOutcome::Wait => continue,
                ClaimOutcome::Parked { op_id, file_id, reason } => {
                    log_parked(&op_id, file_id.as_deref(), reason);
                    outcome.retried_op_ids.push(op_id);
                    continue;
                }
                ClaimOutcome::WaitAfterHandOver(took_over) => {
                    self.after_hand_over(&op, &took_over);
                    continue;
                }
                ClaimOutcome::Claimed(claimed) => {
                    if let Some(took_over) = &claimed.took_over {
                        self.after_hand_over(&claimed.op, took_over);
                    }
                    claimed
                }
            };
```

with

```rust
    /// The hand-over committed: one line, then the retired copy is unlinked (S6).
    fn after_hand_over(&self, successor: &PendingOperation, took_over: &TookOver) {
        log_took_over(&successor.op_id, successor.file_id.as_deref(), &took_over.parked_op_id);
        if let Some(path) = took_over.released_payload.as_deref()
            && let Err(e) = crate::staged_payload::remove(&self.db, Path::new(path))
        {
            tracing::warn!(error = %e, "staged upload cleanup deferred; journal retained");
        }
    }
```

4.6 Mock (`versioned_server_answer`, the `POST /api/v1/uploads/init` branch, before the stale-base check): `if let Some(message) = body["file_id"].as_str().and_then(|id| s.conflict_init_once.remove(id)) { s.inits.push((body, 409)); return http_json("409 Conflict", serde_json::json!({ "error": message })); }`.

- [ ] **Step 5: The hand-over in the claim (`state_db.rs`, §8.4, S3 step 4)**

Add `took_over: Option<TookOver>` to `ClaimedOp`, `TookOver` and `ClaimOutcome::WaitAfterHandOver(TookOver)`. Replace Task 3's step 4 with a loop that may hand over once per parked predecessor:

```rust
        let mut write = finder_write_conn(&tx, op_id)?;
        let mut took_over: Option<TookOver> = None;
        while let Some(current) = write.clone() {
            if current.base_pending > 0 {
                return finish_wait(tx, took_over); // step 3
            }
            let Some(predecessor) = current.after_write_id.clone() else { break };
            let pred = predecessor_conn(&tx, &predecessor)?;
            match pred {
                None => return park_in_claim(tx, op_id, &op, ParkReason::PredecessorLost, now), // S5
                Some(pred) if pred.attempts < pred.max_attempts => return finish_wait(tx, took_over),
                Some(pred) if pred.origin != Some(WriteOrigin::Minted) => {
                    return park_in_claim(tx, op_id, &op, ParkReason::PredecessorParked, now); // m-2
                }
                // A recorded completion never parks (§8.6 rule 6, Task 7); wait for its landing.
                Some(pred) if pred.completed => return finish_wait(tx, took_over),
                Some(pred) => {
                    hand_over_conn(&tx, op_id, &current.write_id, &op, &pred)?;
                    took_over = Some(TookOver { parked_op_id: pred.op_id.clone(), released_payload: pred.payload_path.clone() });
                    write = finder_write_conn(&tx, op_id)?; // N now carries W's base, after_write_id or base_pending
                }
            }
        }
```

`finish_wait(tx, took_over)` commits and returns `Wait` when `took_over` is `None`, else `WaitAfterHandOver(took_over)`. `park_in_claim` is Task 3's park statement plus commit, returning `ClaimOutcome::Parked`. The `op` used by the final `Claimed` is re-read after a hand-over (`SELECT {PENDING_OPERATION_COLUMNS} … WHERE op_id = ?1`), and `took_over` rides on it.

```rust
struct Predecessor {
    op_id: String,
    kind: String,
    parent_id: Option<String>,
    target_path: Option<String>,
    metadata_json: Option<String>,
    payload_path: Option<String>,
    base_version: Option<i64>,
    base_object_version_id: Option<String>,
    after_write_id: Option<String>,
    base_pending: i64,
    attempts: i64,
    max_attempts: i64,
    origin: Option<WriteOrigin>,
    completed: bool,
}

fn predecessor_conn(conn: &Connection, write_id: &str) -> Result<Option<Predecessor>> {
    conn.query_row(
        "SELECT q.op_id, q.kind, q.parent_id, q.target_path, q.metadata_json, q.payload_path,
                q.base_version, q.base_object_version_id, q.after_write_id, q.base_pending,
                q.attempts, q.max_attempts, q.write_origin,
                EXISTS (SELECT 1 FROM upload_resume r WHERE r.op_id = q.op_id AND r.completed_version IS NOT NULL)
         FROM operation_queue q WHERE q.write_id = ?1",
        params![write_id],
        |row| {
            Ok(Predecessor {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                parent_id: row.get(2)?,
                target_path: row.get(3)?,
                metadata_json: row.get(4)?,
                payload_path: row.get(5)?,
                base_version: row.get(6)?,
                base_object_version_id: row.get(7)?,
                after_write_id: row.get(8)?,
                base_pending: row.get(9)?,
                attempts: row.get(10)?,
                max_attempts: row.get(11)?,
                origin: WriteOrigin::from_db(row.get::<_, Option<String>>(12)?.as_deref()),
                completed: row.get(13)?,
            })
        },
    )
    .optional()
}

/// §8.4: N takes W's role. N's bytes contain W's (§8.2, last point), so no byte is lost.
fn hand_over_conn(conn: &Connection, n_op_id: &str, n_write_id: &str, n: &PendingOperation, w: &Predecessor) -> Result<()> {
    let w_is_create = w
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
    // Plan Spec issue 1: a Finder create is `upload_version` with `"operation": "create_file"`.
    let (kind, parent_id, target_path, metadata_json) = if w_is_create {
        let mut metadata: serde_json::Value = n
            .metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        metadata["operation"] = serde_json::json!("create_file");
        if let Some(map) = metadata.as_object_mut() {
            map.remove("base_version_identifier");
        }
        (w.kind.clone(), w.parent_id.clone(), w.target_path.clone(), Some(metadata.to_string()))
    } else {
        (n.kind.as_str().to_string(), n.parent_id.clone(), n.target_path.clone(), n.metadata_json.clone())
    };
    let moved = conn.execute(
        "UPDATE operation_queue
         SET base_version = ?3, base_object_version_id = ?4, after_write_id = ?5, base_pending = ?6,
             kind = ?7, parent_id = ?8, target_path = ?9, metadata_json = ?10
         WHERE op_id = ?1 AND write_id = ?2",
        params![
            n_op_id, n_write_id, w.base_version, w.base_object_version_id, w.after_write_id,
            w.base_pending, kind, parent_id, target_path, metadata_json,
        ],
    )?;
    let retired = conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![w.op_id])?;
    if moved != 1 || retired != 1 {
        return Err(rusqlite::Error::StatementChangedRows(moved + retired)); // the transaction rolls back
    }
    conn.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![w.op_id])?;
    if let Some(path) = &w.payload_path {
        conn.execute(
            "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1) ON CONFLICT(path) DO UPDATE SET completed = 1",
            params![path],
        )?;
    }
    Ok(())
}
```

(`OperationKind::as_str` is private today, `SD:259`; make it `pub(crate)`.)

- [ ] **Step 6: GREEN, mutations**

Step-2 command into `$EVID/r4-t5-green.log`; expected `ok. 9 passed` (8 new and the rewritten test). Mutations (`$EVID/r4-t5-mut-<T>.log`):
- T24: classify `StaleBase` as `Other` → retried, "parked after one attempt" fails.
- T25: treat every 409 as `StaleBase` → the op parks, the init summary has one entry.
- T23: treat a missing payload as an ordinary failure (today's `anyhow!`) → N waits through backoff; the pass-count assertion fails.
- T26: drop the `origin != Minted` arm → (b) N takes C's create and uploads.
- T41: implement the hand-over as two separate locked calls (delete N's op, then rewrite W's op with N's write id and payload), with `self.seam("claim:before_tx")` between them → N2 resolves to a numeric base, meets 409 and parks; the latest bytes are N's.
- T67: replace the hand-over arm with Task 3's `PredecessorParked` park → N never attempts; the init summary has one entry.
- P8: swap the two constants → the first assertion fails.
- RF4: rule 5 returns `Pending` for a token → nothing parks; the init summary differs.

- [ ] **Step 7: Gate and commit**

Task gate; expected lib count **`L0 + 36`**.

```bash
cd $WT && git commit -m "feat(desktop): stale bases and missing staged copies park at once, and the next save takes over" -- src-tauri/src/api_client.rs src-tauri/src/engine_bridge.rs src-tauri/src/state_db.rs
```

**Stop point:** after the commit.

---

## Task 6: Rule 2, the snapshot side: version 0 and versionless replaces (spec §6.1 rule 6a, §6.3.2–§6.3.4, I-2, I-3; commit 6)

No replace of a known file is sent without a base, a version the server left out is learned from a snapshot, and a phone's versionless replace reaches this Mac's disk.

**Files:**
- Modify: `src-tauri/src/state_db.rs`: `request_resnapshot` (`SD:1733-1741`) becomes a counter; `take_needs_resnapshot` (`SD:1748-1764`) is replaced by `peek_resnapshot_request` + `clear_resnapshot_request`; new `SnapshotVersion`, `apply_snapshot_version`, `has_base_pending_uploads`, `note_snapshot_for_base_pending`, `clear_version_filled`; `engine_start_repair` gains the version-0 request; `accept_finder_write` clears `version_filled`; the test `needs_resnapshot_flag_is_take_once` (`SD:5636-5650`) is rewritten in place
- Modify: `src-tauri/src/engine_bridge.rs`: `sync_tick_outcome` (`EB:5603-5619`); `bootstrap_from_snapshot` (`EB:5701-5713`); `process_metadata_row` (`EB:6261-6400`); `apply_sync_op`'s content arm (`EB:6074-6091`); the landing clears `version_filled` (in `apply_completed_upload`, `EB:1121-1194`); the test at `EB:10529-10532` reads `peek_resnapshot_request`; the mock answers `/api/v1/sync/snapshot` and `/api/v1/sync/ops`
- Test: `engine_bridge.rs` tests, `state_db.rs` tests

**Interfaces:**
- Consumes: `decide_base` rule 6a (Task 3) — it reads `version_filled`, which this task starts setting; `log_parked` (Task 3).
- Produces:
  - `StateDb::request_resnapshot(&self) -> Result<()>` (now increments a counter), `StateDb::peek_resnapshot_request(&self) -> Result<Option<i64>>`, `StateDb::clear_resnapshot_request(&self, seen: i64) -> Result<bool>` (`DELETE … WHERE value = seen`; `false` when a request arrived meanwhile)
  - `state_db::SnapshotVersion { Unchanged, Filled { resolved_ops: Vec<String> }, Raised { old: i64, new: i64 }, SkippedUploading }`
  - `StateDb::apply_snapshot_version(&self, file_id: &str, node_version: i64, node_size: i64, node_is_uploading: bool) -> Result<SnapshotVersion>` (S1.5: one transaction per row)
  - `StateDb::has_base_pending_uploads(&self) -> Result<bool>`; `StateDb::note_snapshot_for_base_pending(&self, now: i64) -> Result<Vec<(String, Option<String>)>>` (the ops it parked: `(op_id, file_id)`); `StateDb::clear_version_filled(&self, file_id: &str) -> Result<()>`
  - `EngineStartRepair.resnapshot_requested: bool`
  - mock: `VersionedServerState.snapshots: VecDeque<(String, serde_json::Value)>` answered by `GET /api/v1/sync/snapshot` in order (`503` when empty); `GET /api/v1/sync/ops?since=N` answers `{"ops": [], "since": N}`

- [ ] **Step 1: Write the failing tests**

Rewrite `needs_resnapshot_flag_is_take_once` (`SD:5636-5650`) in place as `a_resnapshot_request_survives_a_request_made_while_it_runs`:

```rust
    #[test]
    fn a_resnapshot_request_survives_a_request_made_while_it_runs() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
        db.request_resnapshot().unwrap();
        let seen = db.peek_resnapshot_request().unwrap().expect("requested");
        db.request_resnapshot().unwrap(); // a request made during the bootstrap
        assert!(!db.clear_resnapshot_request(seen).unwrap(), "the newer request survives");
        let again = db.peek_resnapshot_request().unwrap().expect("still requested");
        assert!(db.clear_resnapshot_request(again).unwrap());
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);
    }
```

Add T52 to `state_db.rs` tests:

```rust
    #[test]
    fn engine_start_requests_a_snapshot_for_version_zero_rows() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "versioned", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("versioned").unwrap().unwrap();
        contract.current_version = 4;
        db.set_file_contract_state(&contract).unwrap();
        assert!(!db.engine_start_repair().unwrap().resnapshot_requested);
        assert_eq!(db.peek_resnapshot_request().unwrap(), None);

        seed_own_row(&db, "version-zero", FileStatus::Local, 20); // learned from a legacy file_create op
        assert!(db.engine_start_repair().unwrap().resnapshot_requested);
        assert!(db.peek_resnapshot_request().unwrap().is_some());
    }
```

In `engine_bridge.rs` tests, two helpers, then the tests (setup as in Task 3):

```rust
    /// A row this Mac learned from a legacy `file_create` op: at version 0 (FILES:2286-2297).
    fn seed_version_zero_row(bridge: &EngineBridge, server: &VersionedServerMock, file_id: &str, status: FileStatus) {
        seed_uploaded_row(bridge, server, file_id);
        let mut contract = bridge.db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = 0;
        bridge.db.set_file_contract_state(&contract).unwrap();
        bridge.db.set_status(file_id, status).unwrap();
    }

    fn node(master_key: &[u8; 32], id: &str, version: i64, is_uploading: bool) -> serde_json::Value {
        let mut node = snap_node(master_key, id, "notes.txt", None, false, 0);
        node["version_number"] = serde_json::json!(version);
        node["is_uploading"] = serde_json::json!(is_uploading);
        node
    }

    #[tokio::test]
    async fn i2_a_row_without_a_version_never_uploads_without_a_base() {
        // master_key [72u8; 32]
        seed_version_zero_row(&bridge, &server, "legacy", FileStatus::Local);
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "legacy", "notes.txt", b"edit", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        assert!(server.state.lock().unwrap().inits.is_empty(), "no init until the version is known");
        server.state.lock().unwrap().snapshots.push_back(("200 OK".into(), serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "legacy", 1, false)] })));
        let logs = capture_logs_async(async {
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("legacy"), json!(1), 201)]);
        assert!(logs.contains("queued write based on the version the snapshot reported"), "{logs}");
    }

    #[tokio::test]
    async fn a_versionless_create_op_requests_a_snapshot_that_fills_local_rows() {
        // master_key [73u8; 32]
        let create = crate::api_client::SyncOp {
            seq_id: 5,
            op_type: "file_create".into(),
            payload: serde_json::json!({ "id": "phone-file", "name_encrypted": enc_name(&master_key, "phone-file", "notes.txt"), "parent_id": null, "size_bytes": 10 }),
        };
        apply_sync_op(&bridge, &sync_root, &create, now_secs(), &mut Vec::new()).unwrap();
        assert!(bridge.db.peek_resnapshot_request().unwrap().is_some(), "a versionless op asks for a snapshot");
        bridge.db.set_status("phone-file", FileStatus::Local).unwrap();
        let snapshot = crate::api_client::SyncSnapshot { seq_id: 6, nodes: vec![node(&master_key, "phone-file", 1, false)] };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        let presentation = bridge.db.item_presentation("phone-file").unwrap().unwrap();
        assert_eq!(presentation.contract.current_version, 1, "filled although the row is Local");
        assert!(presentation.version_filled);
        assert_eq!(bridge.db.get_file("phone-file").unwrap().unwrap().status, FileStatus::Local, "content untouched");
        drop(server.finish());
    }

    #[tokio::test]
    async fn i2_a_zero_base_save_after_the_fill_lands_on_the_filled_version() {
        // master_key [74u8; 32]; rule 6a
        seed_version_zero_row(&bridge, &server, "filled", FileStatus::Local);
        let snapshot = crate::api_client::SyncSnapshot { seq_id: 2, nodes: vec![node(&master_key, "filled", 1, false)] };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        fp_save(&bridge, dir.path(), "filled", "notes.txt", b"saved before the re-read", "0");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("filled"), json!(1), 201)]);
        assert!(bridge.db.list_operations_for_file("filled").unwrap().is_empty(), "nothing parks");
    }

    #[tokio::test]
    async fn i2_a_failed_snapshot_keeps_the_request_and_base_pending_resolves_on_the_next_success() {
        // master_key [75u8; 32]
        seed_version_zero_row(&bridge, &server, "flaky-snapshot", FileStatus::Local);
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "flaky-snapshot", "notes.txt", b"edit", "0");
        {
            let mut s = server.state.lock().unwrap();
            s.snapshots.push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
            s.snapshots.push_back(("200 OK".into(), serde_json::json!({ "seq_id": 3, "nodes": [node(&master_key, "flaky-snapshot", 1, false)] })));
        }
        // The runner runs the queue only after an Ok tick (RUN:1293-1315).
        assert!(sync_tick_outcome(&bridge, &sync_root).await.is_err(), "the failed snapshot fails the tick");
        assert!(bridge.db.peek_resnapshot_request().unwrap().is_some(), "the request survives a failed snapshot");
        sync_tick_outcome(&bridge, &sync_root).await.unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("flaky-snapshot"), json!(1), 201)]);
        assert_eq!(bridge.db.peek_resnapshot_request().unwrap(), None);
    }

    #[tokio::test]
    async fn i2_the_fill_reaches_an_uploading_row() {
        // master_key [76u8; 32]
        seed_version_zero_row(&bridge, &server, "uploading-zero", FileStatus::Local);
        fp_save(&bridge, dir.path(), "uploading-zero", "notes.txt", b"edit", "0");
        assert_eq!(bridge.db.get_file("uploading-zero").unwrap().unwrap().status, FileStatus::Uploading);
        let snapshot = crate::api_client::SyncSnapshot { seq_id: 2, nodes: vec![node(&master_key, "uploading-zero", 1, false)] };
        apply_snapshot(&bridge, &sync_root, &snapshot, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(bridge.db.get_file_contract_state("uploading-zero").unwrap().unwrap().current_version, 1);
        let op = bridge.db.list_operations_for_file("uploading-zero").unwrap().remove(0);
        assert_eq!(op.base_version, Some(1), "the op is based on the filled version");
        assert_eq!(bridge.db.finder_write(&op.op_id).unwrap().unwrap().base_pending, 0);
        drop(server.finish());
    }

    #[tokio::test]
    async fn i3_a_versionless_replace_from_another_device_reaches_the_disk() {
        // master_key [77u8; 32]
        let token = land_one_save(&bridge, &server, dir.path(), &sync_root, "replaced").await;
        let replace = crate::api_client::SyncOp {
            seq_id: 9,
            op_type: "file_create".into(),
            payload: serde_json::json!({ "id": "replaced", "name_encrypted": enc_name(&master_key, "replaced", "notes.txt"), "parent_id": null, "size_bytes": 12 }),
        };
        apply_sync_op(&bridge, &sync_root, &replace, now_secs(), &mut Vec::new()).unwrap();
        let seen = bridge.db.peek_resnapshot_request().unwrap().expect("requested");
        assert_eq!(held_content_version(&bridge, "replaced"), token, "the op alone cannot tell a replace");
        // The legacy init bumped the version before the bytes exist: skipped, the request stays.
        let mid = crate::api_client::SyncSnapshot { seq_id: 10, nodes: vec![node(&master_key, "replaced", 3, true)] };
        apply_snapshot(&bridge, &sync_root, &mid, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        assert_eq!(held_content_version(&bridge, "replaced"), token);
        assert!(!bridge.db.clear_resnapshot_request(seen).unwrap(), "the skip kept the request");
        let done = crate::api_client::SyncSnapshot { seq_id: 11, nodes: vec![node(&master_key, "replaced", 3, false)] };
        let logs = capture_logs_async(async {
            apply_snapshot(&bridge, &sync_root, &done, now_secs(), now_secs(), &mut Vec::new()).unwrap();
        })
        .await;
        assert_eq!(held_content_version(&bridge, "replaced"), "3", "the system re-downloads the phone's bytes");
        assert!(logs.contains("file version raised from the snapshot"), "{logs}");
        drop(server.finish());
    }

    #[tokio::test]
    async fn base_pending_parks_after_ten_successful_snapshots_without_the_file() {
        // master_key [78u8; 32]
        seed_version_zero_row(&bridge, &server, "never-listed", FileStatus::Local);
        seed_uploaded_row(&bridge, &server, "listed");
        bridge.db.set_sync_cursor(0).unwrap();
        fp_save(&bridge, dir.path(), "never-listed", "notes.txt", b"edit", "0");
        let listed = serde_json::json!({ "seq_id": 1, "nodes": [node(&master_key, "listed", 1, false)] });
        let op_id = bridge.db.list_operations_for_file("never-listed").unwrap().remove(0).op_id;
        for success in 1..=10 {
            {
                let mut s = server.state.lock().unwrap();
                s.snapshots.push_back(("503 Service Unavailable".into(), serde_json::json!({ "error": "busy" })));
                s.snapshots.push_back(("200 OK".into(), listed.clone()));
            }
            assert!(sync_tick_outcome(&bridge, &sync_root).await.is_err(), "a failed snapshot costs nothing");
            sync_tick_outcome(&bridge, &sync_root).await.unwrap();
            let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
            if success < 10 {
                assert!(op.attempts < op.max_attempts, "not parked after {success} successful snapshots");
            } else {
                assert_eq!(op.attempts, op.max_attempts, "parked base_unknown after the 10th");
            }
        }
        let state = server.finish();
        assert!(state.inits.is_empty());
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- a_resnapshot_request_survives engine_start_requests_a_snapshot i2_a_row_without_a_version a_versionless_create_op i2_a_zero_base_save_after i2_a_failed_snapshot i2_the_fill_reaches i3_a_versionless_replace base_pending_parks_after > $EVID/r4-t6-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t6-red.log | head -20
```

Expected REDs: compile errors for the counter API; then T11 never resolves (the op waits forever: nothing fills `base_pending`); T13 the `Local` row stays at 0 (`EB:6326` short-circuit); T45 parks (`version_filled` never set, rule 6a′); T46 the request is consumed by the failed tick (`EB:5605`); T47 branch 3 skips the `Uploading` row (`EB:6383-6392`); T48 the content version stays the token; T52 nothing requested; T53 never parks.

- [ ] **Step 3: The request counter (`state_db.rs`)**

```rust
    pub fn request_resnapshot(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        request_resnapshot_conn(&conn)
    }

    /// The pending request's counter, or `None`. Read at the start of a tick; the
    /// request is cleared only after the snapshot succeeded (§6.3.2, I-2(b)).
    pub fn peek_resnapshot_request(&self) -> Result<Option<i64>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM sync_state WHERE key = ?1",
            params![Self::NEEDS_RESNAPSHOT_KEY],
            |row| row.get(0),
        )
        .optional()
    }

    /// Clear the request the tick read; a request made meanwhile survives.
    pub fn clear_resnapshot_request(&self, seen: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "DELETE FROM sync_state WHERE key = ?1 AND CAST(value AS INTEGER) = ?2",
            params![Self::NEEDS_RESNAPSHOT_KEY, seen],
        )?;
        Ok(n == 1)
    }
```

with

```rust
fn request_resnapshot_conn<C: std::ops::Deref<Target = Connection>>(conn: &C) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
        params![StateDb::NEEDS_RESNAPSHOT_KEY],
    )?;
    Ok(())
}
```

Delete `take_needs_resnapshot`; its callers become `peek_resnapshot_request` (`EB:5605`, and the test at `EB:10529-10532`, which asserts `.is_some()`).

- [ ] **Step 4: Fill, raise and `base_pending` (`state_db.rs`, S1.5)**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotVersion {
    Unchanged,
    /// The row was at 0: filled, `version_filled = 1`, and these ops got their base.
    Filled { resolved_ops: Vec<String> },
    /// A newer version than the row's (I-3): the token clears through the predicate.
    Raised { old: i64, new: i64 },
    /// The node is mid-upload: the legacy init bumps the version first (FILES:2814-2830).
    SkippedUploading,
}

    pub fn apply_snapshot_version(&self, file_id: &str, node_version: i64, node_size: i64, node_is_uploading: bool) -> Result<SnapshotVersion> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(String, i64)> = tx
            .query_row("SELECT status, current_version FROM files WHERE file_id = ?1", params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((status, current)) = row else { return Ok(SnapshotVersion::Unchanged) };
        // A Conflict row owns its transitions (EB:6384-6386); a Trashing row is leaving.
        if status == FileStatus::Trashing.as_str() || status == FileStatus::Conflict.as_str() {
            return Ok(SnapshotVersion::Unchanged);
        }
        let fill = current == 0 && node_version > 0;
        let raise = current > 0 && node_version > current;
        if !fill && !raise {
            return Ok(SnapshotVersion::Unchanged);
        }
        if node_is_uploading {
            request_resnapshot_conn(&tx)?; // the clear at the end of this tick then fails: the request stays
            tx.commit()?;
            return Ok(SnapshotVersion::SkippedUploading);
        }
        let write_queued: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND write_id IS NOT NULL
                            AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
            |r| r.get(0),
        )?;
        // On a row with a queued write only the version changes: the size is that write's (§6.3.2).
        if write_queued {
            tx.execute("UPDATE files SET current_version = ?2, local_base_version = ?2 WHERE file_id = ?1", params![file_id, node_version])?;
        } else {
            tx.execute(
                "UPDATE files SET current_version = ?2, local_base_version = ?2, size_bytes = ?3 WHERE file_id = ?1",
                params![file_id, node_version, node_size],
            )?;
        }
        let outcome = if fill {
            tx.execute("UPDATE files SET version_filled = 1 WHERE file_id = ?1", params![file_id])?;
            let resolved_ops = {
                let mut stmt = tx.prepare("SELECT op_id FROM operation_queue WHERE file_id = ?1 AND base_pending > 0 ORDER BY rowid")?;
                let rows = stmt.query_map(params![file_id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>>>()?
            };
            tx.execute(
                "UPDATE operation_queue SET base_version = ?2, base_pending = 0 WHERE file_id = ?1 AND base_pending > 0",
                params![file_id, node_version],
            )?;
            SnapshotVersion::Filled { resolved_ops }
        } else {
            // The snapshot carries no object version id (SY:197); the old one is no longer current.
            tx.execute(
                "UPDATE files SET current_object_version_id = NULL, version_filled = 0 WHERE file_id = ?1",
                params![file_id],
            )?;
            SnapshotVersion::Raised { old: current, new: node_version }
        };
        record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        tx.commit()?;
        Ok(outcome)
    }

    pub fn has_base_pending_uploads(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT EXISTS(SELECT 1 FROM operation_queue WHERE base_pending > 0)", [], |r| r.get(0))
    }

    /// After a successful snapshot: one more miss for every op still waiting; park at the 10th
    /// (§6.3.3; plan Spec issue 3: `base_pending` = 1 + misses).
    pub fn note_snapshot_for_base_pending(&self, now: i64) -> Result<Vec<(String, Option<String>)>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE operation_queue SET base_pending = base_pending + 1 WHERE base_pending > 0", [])?;
        let parked = {
            let mut stmt = tx.prepare("SELECT op_id, file_id FROM operation_queue WHERE base_pending >= 11 ORDER BY rowid")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.execute(
            "UPDATE operation_queue
             SET base_pending = 0, attempts = max_attempts, last_error = 'base_unknown',
                 last_error_class = 'base_unknown', updated_at = ?1
             WHERE base_pending >= 11",
            params![now],
        )?;
        tx.commit()?;
        Ok(parked)
    }

    pub fn clear_version_filled(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("UPDATE files SET version_filled = 0 WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }
```

`accept_finder_write`'s `Modify` branch adds `UPDATE files SET version_filled = 0 WHERE file_id = ?1` after `record_local_write_conn` (a content write touches the row). `engine_start_repair` adds, inside its transaction:

```rust
        let version_zero: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM files f
              WHERE f.current_version = 0 AND f.item_kind = 'file' AND f.namespace = 'my_files'
                AND f.status != 'trashing'
                AND NOT EXISTS (SELECT 1 FROM operation_queue q
                                WHERE q.file_id = f.file_id
                                  AND json_extract(q.metadata_json, '$.operation') = 'create_file'))",
            [],
            |r| r.get(0),
        )?;
        if version_zero {
            request_resnapshot_conn(&tx)?;
        }
```

and returns `resnapshot_requested: version_zero`.

- [ ] **Step 5: The engine hooks (`engine_bridge.rs`)**

5.1 `sync_tick_outcome` (`EB:5605`):

```rust
    // §6.3.2: while any base_pending upload exists, every pass asks for a snapshot.
    if bridge.db().has_base_pending_uploads()? {
        bridge.db().request_resnapshot()?;
    }
    let resnapshot_request = bridge.db().peek_resnapshot_request()?;
```

In both bootstrap arms (`EB:5609-5620`), after `bootstrap_from_snapshot(...).await?` succeeds: `if let Some(seen) = resnapshot_request { bridge.db().clear_resnapshot_request(seen)?; }`. A failed bootstrap returns its error before that line, so the request survives and the queue does not run in that pass (`RUN:1293-1315`). The second arm's guard becomes `Some(_) if resnapshot_request.is_some()`.

5.2 `bootstrap_from_snapshot` (`EB:5711-5712`): after `apply_snapshot(...)` returns `Ok(applied)`:

```rust
    for (op_id, file_id) in bridge.db().note_snapshot_for_base_pending(now_secs)? {
        log_parked(&op_id, file_id.as_deref(), ParkReason::BaseUnknown);
    }
    Ok(applied)
```

5.3 `process_metadata_row` (`EB:6274`), after `let remote_updated = …`:

```rust
    if source == RowSource::Snapshot
        && let Some(node_version) = f["version_number"].as_i64()
    {
        let is_uploading = f["is_uploading"].as_bool() == Some(true);
        match bridge.db().apply_snapshot_version(file_id, node_version, size, is_uploading)? {
            crate::state_db::SnapshotVersion::Raised { old, new } => tracing::warn!(
                file_id = %file_id, old_version = old, new_version = new, "file version raised from the snapshot"
            ),
            crate::state_db::SnapshotVersion::Filled { resolved_ops } => {
                for op_id in resolved_ops {
                    tracing::warn!(op_id = %op_id, file_id = %file_id, base_version = node_version,
                        "queued write based on the version the snapshot reported");
                }
            }
            crate::state_db::SnapshotVersion::Unchanged | crate::state_db::SnapshotVersion::SkippedUploading => {}
        }
    }
```

The `EB:6326` short-circuit stays for every other field.

5.4 `apply_sync_op`'s content arm (`EB:6074`): before `synthesize_op_row`, `let versionless = op.op_type != "folder_create" && payload["version_number"].as_i64().is_none();`; after `process_metadata_row`, `if versionless { bridge.db().request_resnapshot()?; }` (plan Spec issue 9). For `file_update` and `file_create` on an existing row, also `bridge.db().clear_version_filled(id)?;` (a content op touched the row, §6.1).

5.5 The landing: in `apply_completed_upload` after `set_file_contract_state` (`EB:1187`), `self.db.clear_version_filled(server_file_id)?;` (Task 7 folds it into the landing transaction).

5.6 The mock: in `versioned_server_answer`, before the final 404: `GET /api/v1/sync/snapshot` pops `s.snapshots` (`http_json(&status, body)`, or `503 Service Unavailable` when empty); `GET` paths starting with `/api/v1/sync/ops` answer `{"ops": [], "since": <the since value>}`.

- [ ] **Step 6: GREEN, mutations**

Step-2 command into `$EVID/r4-t6-green.log`; expected `ok. 9 passed` (8 new and the rewritten flag test). Mutations (`$EVID/r4-t6-mut-<T>.log`):
- T11: let the claim ignore `base_pending` → an `init` without a base is attempted, the guard parks it, and the init summary is empty after the snapshot.
- T13: run the fill only for rows whose status is not `Local` (keep the short-circuit) → "filled although the row is Local" fails.
- T45: treat `version_filled = 1` like rule 6a′ → the save parks.
- T46: clear the request before the bootstrap (today's order) → "the request survives a failed snapshot" fails.
- T47: fill `Local` rows only → the op's base stays `Some(0)`.
- T48: drop the `versionless` request for `file_create` on an existing row → `peek` is `None` at "requested".
- T52: drop the engine-start check → `resnapshot_requested` is false.
- T53: count passes instead of successful snapshots (increment in `sync_tick_outcome` before the bootstrap) → parked after the 5th success.
- flag test: make `clear_resnapshot_request` delete unconditionally → "the newer request survives" fails.

- [ ] **Step 7: Gate and commit**

Task gate; expected lib count **`L0 + 44`**.

```bash
cd $WT && git commit -m "feat(desktop): learn a missing or raised version from the snapshot before a save is sent" -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs
```

**Stop point:** after the commit.

---

## Task 7: M1: the landing is one transaction after a recorded completion (spec §8.6 rules 1–4 and 6, §8.7 S1.2, §7.1, §9.2–§9.3; commit 7)

A landing can no longer half-apply. A failure after `complete` is retried locally without the network, never duplicates the file, and never trashes a completed one.

**Files:**
- Modify: `src-tauri/src/state_db.rs`: `UploadResume` (`SD:694-711`) gains the completion fields and `get_upload_resume` (`SD:3192-3217`) reads them; connection-level twins `set_file_contract_state_conn` (`SD:2226-2300`) and `delete_file_conn` (`SD:1127-1142`); new `record_completion_claimed`, `LandingInput`, `LandingOutcome`, `apply_landing`; test-only `fail_next_landings_for_test`, `alias_target_for_test`, `set_target_path_for_test`
- Modify: `src-tauri/src/engine_bridge.rs`: `do_upload_version` (`EB:697-890`) records the completion, lands through `apply_landing`, and lands a recorded completion without the network; `apply_completed_upload` (`EB:1121-1194`), `chain_queued_ops_after_upload` (`EB:1216-1258`) and Task 3's interim calls are folded into `apply_landing`; `execute_operation` returns `OpDone`; `process_due_operations`' give-up for a recorded completion; the mock gains `delay_complete` and `DELETE /api/v1/files/{id}`
- Test: `engine_bridge.rs` tests

**Interfaces:**
- Consumes: the claim and `finish_claimed` (Task 2); held columns, `resolve_successors` semantics (Task 3); `clear_version_filled` (Task 6); `metadata_rekeyed_to` (`EB:4031-4056`).
- Produces:
  - `UploadResume { …, completed_version: Option<i64>, completed_object_version_id: Option<String> }` (the 5 `UploadResume {` literals gain `None, None`)
  - `StateDb::record_completion_claimed(&self, op_id: &str, claim_id: &str, version: i64, object_version_id: &str) -> Result<bool>`
  - `state_db::LandingInput<'a> { op_id, claim_id: Option<&'a str>, write_id: Option<&'a str>, local_file_id, server_file_id, target_path: Option<&'a str>, parent_id: Option<&'a str>, landed_base: Option<i64>, produced_version: i64, produced_object_version_id: &'a str, size_bytes: i64, content_type: Option<&'a str>, release_payload: Option<&'a str>, now: i64 }` (`&'a str` where not marked)
  - `state_db::LandingOutcome { pub parked_successors: Vec<(String, ParkReason)>, pub resolved_successors: usize }`
  - `StateDb::apply_landing(&self, input: &LandingInput<'_>, rekey: &dyn Fn(&PendingOperation, &str) -> anyhow::Result<Option<String>>) -> Result<Option<LandingOutcome>>` (`None`: the op moved since the claim; nothing was written)
  - `engine_bridge::OpDone { Remove { release: Option<String> }, Removed { release: Option<String> } }`; `execute_operation` returns `anyhow::Result<OpDone>`: `Removed` when the landing transaction already deleted the op
  - `fn log_completed_landing_retried(op_id: &str, file_id: Option<&str>, attempt: i64)` → `warn!` "upload completed on the server; local landing will be retried"
  - mock: `VersionedServerState.delay_complete: HashMap<String, Duration>` (by session), `trashes: Vec<String>` (`DELETE /api/v1/files/{id}` answers `200 {"ok": true}`)

- [ ] **Step 1: Write the failing tests** (setup as in Task 3)

```rust
    #[tokio::test]
    async fn m1_a_chain_failure_after_complete_never_duplicates() {
        // master_key [79u8; 32]
        let created = fp_create(&bridge, dir.path(), "new.txt", b"created");
        let provisional = created.outcome_file_id();
        fp_save(&bridge, dir.path(), &provisional, "new.txt", b"created, edited", created.token.as_deref().unwrap());
        // The successor's name cannot be re-encrypted for the server id: no display name, no path.
        let successor = bridge.db.list_operations_for_file(&provisional).unwrap().remove(1);
        bridge.db.set_target_path_for_test(&successor.op_id, None);
        let logs = capture_logs_async(async { drain_upload_queue(&bridge, &sync_root).await; }).await;
        let state = server.finish();
        assert_eq!(state.files_with_content().len(), 1, "one server file: {:?}", state.init_summary());
        assert_eq!(state.inits.len(), 1, "no second init");
        let server_id = state.files_with_content()[0].clone();
        assert!(bridge.db.get_file(&provisional).unwrap().is_none(), "the landing was applied");
        assert!(bridge.db.get_file(&server_id).unwrap().is_some());
        let parked = bridge.db.get_operation(&successor.op_id).unwrap().unwrap();
        assert_eq!(parked.attempts, parked.max_attempts, "the successor parked with its bytes");
        assert!(std::path::Path::new(parked.payload_path.as_deref().unwrap()).is_file());
        assert!(logs.contains("rekey_failed"), "{logs}");
    }

    #[tokio::test]
    async fn i1_a_landing_with_a_later_write_queued_keeps_it_uploading() {
        // master_key [80u8; 32]
        seed_uploaded_row(&bridge, &server, "kept-up");
        let a = fp_save(&bridge, dir.path(), "kept-up", "notes.txt", b"A", "1");
        let b = fp_save(&bridge, dir.path(), "kept-up", "notes.txt", b"A, and B's longer bytes", a.token.as_deref().unwrap());
        server.state.lock().unwrap().fail_first_chunk_once.insert("session-2".into()); // B backs off
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        let entry = bridge.db.get_file("kept-up").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_eq!(payload.status, "uploading");
        assert_eq!(payload.size_bytes, b"A, and B's longer bytes".len() as i64, "the newer write's size");
        assert_eq!(payload.content_version, b.token, "the newer write's token");
        drop(server.finish());
    }

    #[tokio::test]
    async fn landing_vs_new_save_the_chain_step_sees_every_successor() {
        // master_key [81u8; 32]
        let bridge = Arc::new(bridge);
        server.state.lock().unwrap().delay_complete.insert("session-1".into(), Duration::from_millis(400));
        seed_uploaded_row(&bridge, &server, "landing-race");
        let w = fp_save(&bridge, dir.path(), "landing-race", "notes.txt", b"W", "1").token.unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (saver, save_dir, w_token, seen_at_seam) = (Arc::clone(&bridge), dir.path().to_path_buf(), w.clone(), Arc::clone(&seen));
        bridge.seams.arm("landing:after_complete", move || {
            run_competing(move || {
                seen_at_seam.lock().unwrap().push(held_content_version(&saver, "landing-race"));
                fp_save(&saver, &save_dir, "landing-race", "notes.txt", b"W N", &w_token);
                seen_at_seam.lock().unwrap().push(held_content_version(&saver, "landing-race"));
            });
        });
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(
            state.init_summary(),
            vec![(json!("landing-race"), json!(1), 201), (json!("landing-race"), json!(2), 201)],
            "N lands on W's produced version"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[0], w, "at the seam: W's token");
        let n = held_content_version(&bridge, "landing-race");
        assert_eq!(seen[1], n, "after N's accept: N's token, kept through the landing");
        assert!(seen.iter().chain(std::iter::once(&n)).all(|v| crate::write_token::parse_token(v).is_some()), "never a numeric content version: {seen:?}");
    }

    #[tokio::test]
    async fn a_completed_session_is_never_abandoned_at_give_up() {
        // master_key [82u8; 32]; m-11
        let created = fp_create(&bridge, dir.path(), "completed.txt", b"bytes the server has");
        let provisional = created.outcome_file_id();
        crate::state_db::fail_next_landings_for_test(30);
        let mut now = now_secs();
        let logs = capture_logs_async(async {
            for _ in 0..26 {
                bridge.process_due_operations(&sync_root, now).await.unwrap();
                now += 10_000;
            }
        })
        .await;
        let op = bridge.db.list_operations_for_file(&provisional).unwrap().remove(0);
        assert!(op.attempts < op.max_attempts, "never parked");
        assert!(bridge.db.get_upload_resume(&op.op_id).unwrap().unwrap().completed_version.is_some(), "the resume row is kept");
        assert!(server.state.lock().unwrap().trashes.is_empty(), "the completed server file is never trashed");
        assert_eq!(server.state.lock().unwrap().inits.len(), 1, "no second upload");
        assert!(logs.matches("local landing will be retried").count() >= 25, "{logs}");
        crate::state_db::fail_next_landings_for_test(0);
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        assert!(bridge.db.get_file(&provisional).unwrap().is_none(), "landed once the disk recovers");
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_recorded_completion_is_applied_without_the_network() {
        // master_key [83u8; 32]; §8.6 rule 4
        seed_uploaded_row(&bridge, &server, "recorded");
        fp_save(&bridge, dir.path(), "recorded", "notes.txt", b"edit", "1");
        crate::state_db::fail_next_landings_for_test(1);
        let now = now_secs();
        bridge.process_due_operations(&sync_root, now).await.unwrap();
        let before = server.state.lock().unwrap().requests.len();
        bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
        let state = server.finish();
        assert_eq!(state.requests.len(), before, "the retry asked the server nothing: {:?}", &state.requests[before..]);
        assert!(bridge.db.list_operations_for_file("recorded").unwrap().is_empty(), "landed");
        assert_eq!(bridge.db.get_file_contract_state("recorded").unwrap().unwrap().current_version, 2);
    }

    #[tokio::test]
    async fn the_create_landing_writes_the_alias_in_its_transaction() {
        // master_key [84u8; 32]
        let created = fp_create(&bridge, dir.path(), "aliased.txt", b"created");
        let provisional = created.outcome_file_id();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        let server_id = state.files_with_content()[0].clone();
        assert_eq!(bridge.db.alias_target_for_test(&provisional), Some(server_id.clone()));
        assert!(bridge.db.get_file(&provisional).unwrap().is_none());
        assert_eq!(held_content_version(&bridge, &server_id), created.token.unwrap(), "S carries P's create token (§5.4 row 14)");
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- m1_a_chain_failure i1_a_landing_with_a_later landing_vs_new_save a_completed_session_is_never a_recorded_completion_is_applied the_create_landing_writes_the_alias > $EVID/r4-t7-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t7-red.log | head -20
```

Expected: compile errors for `fail_next_landings_for_test`, `delay_complete`, `trashes`, `alias_target_for_test`, `set_target_path_for_test`; then T27 two inits and two server files (the chain's `?`, `EB:828`, fails the landing after `complete`; the retry's `complete` is gone, and the abandon trashes and re-creates, `EB:859-866`); T30 `local` and A's size; T61 a trash request at give-up (`EB:1040-1055`); P9 the retry re-runs the upload; P10 no alias. T40 may already pass, because Task 3's interim landing resolves by write id after the seam; its RED comes from its mutation.

- [ ] **Step 3: Implement `apply_landing` (`state_db.rs`)**

3.1 Extract `set_file_contract_state_conn` and `delete_file_conn` from `SD:2226-2300` and `SD:1127-1142` (generic over `C: Deref<Target = Connection>`; the public methods call them). Add `completed_version` and `completed_object_version_id` to `UploadResume` and to `get_upload_resume`'s `SELECT`.

3.2 Test-only failure injection, a thread-local so parallel tests never share it (the landing runs on the test's own thread under `#[tokio::test]`):

```rust
#[cfg(test)]
thread_local! {
    static FAIL_LANDINGS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// T61, P9: the next `n` landings on this thread fail with a database error.
#[cfg(test)]
pub(crate) fn fail_next_landings_for_test(n: u32) {
    FAIL_LANDINGS.with(|left| left.set(n));
}
```

3.3

```rust
    /// §8.6.1: the completion the server confirmed, recorded before any local bookkeeping.
    pub fn record_completion_claimed(&self, op_id: &str, claim_id: &str, version: i64, object_version_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "UPDATE upload_resume SET completed_version = ?3, completed_object_version_id = ?4
             WHERE op_id = ?1 AND EXISTS (SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2)",
            params![op_id, claim_id, version, object_version_id],
        )?;
        Ok(n == 1)
    }

    /// §8.6.2 and §8.7 S1.2: everything the landing changes, in one transaction.
    pub fn apply_landing(
        &self,
        input: &LandingInput<'_>,
        rekey: &dyn Fn(&PendingOperation, &str) -> anyhow::Result<Option<String>>,
    ) -> Result<Option<LandingOutcome>> {
        #[cfg(test)]
        if FAIL_LANDINGS.with(|left| {
            let n = left.get();
            left.set(n.saturating_sub(1));
            n > 0
        }) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (local, server) = (input.local_file_id, input.server_file_id);
        if let Some(claim_id) = input.claim_id {
            let claimed: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2)",
                params![input.op_id, claim_id],
                |r| r.get(0),
            )?;
            if !claimed {
                return Ok(None);
            }
        }
        // §9.2: Local only when no later write of the file is queued.
        let later_write_queued: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND op_id != ?2
                            AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file'))",
            params![local, input.op_id],
            |r| r.get(0),
        )?;

        // The row and contract (today's apply_completed_upload, EB:1131-1188).
        let mut entry = get_file_conn(&tx, local)?.unwrap_or_else(|| FileEntry {
            file_id: server.to_string(),
            path: input.target_path.unwrap_or(server).to_string(),
            status: FileStatus::Local,
            size_bytes: input.size_bytes,
            modified_at: input.now,
            content_hash: None,
            remote_updated_at: input.now,
            parent_id: input.parent_id.map(str::to_string),
            item_kind: ItemKind::File,
        });
        entry.file_id = server.to_string();
        if let Some(target_path) = input.target_path {
            entry.path = target_path.to_string();
        }
        entry.remote_updated_at = input.now;
        if !later_write_queued {
            entry.status = FileStatus::Local;
            entry.size_bytes = input.size_bytes;
            entry.modified_at = input.now; // the save-time presentation is split off (spec §12)
        }
        upsert_file_conn(&tx, &entry)?;
        let mut contract = get_file_contract_state_conn(&tx, local)?.unwrap_or_else(|| default_contract(server));
        contract.file_id = server.to_string();
        contract.item_kind = ItemKind::File;
        if input.content_type.is_some() {
            contract.content_type = input.content_type.map(str::to_string);
        }
        contract.parent_id = input.parent_id.map(str::to_string);
        contract.current_version = input.produced_version;
        contract.local_base_version = input.produced_version;
        contract.current_object_version_id = Some(input.produced_object_version_id.to_string());
        contract.last_sync_at = input.now;
        set_file_contract_state_conn(&tx, &contract)?;
        tx.execute("UPDATE files SET version_filled = 0 WHERE file_id = ?1", params![server])?;

        let mut outcome = LandingOutcome { parked_successors: Vec::new(), resolved_successors: 0 };
        if local != server {
            // §5.4 row 14: S takes P's held columns; the alias; P's queued ops move to S.
            tx.execute(
                "UPDATE files SET
                    held_write_id = (SELECT held_write_id FROM files WHERE file_id = ?1),
                    held_base = (SELECT held_base FROM files WHERE file_id = ?1),
                    held_version = (SELECT held_version FROM files WHERE file_id = ?1),
                    held_object_version_id = (SELECT held_object_version_id FROM files WHERE file_id = ?1)
                 WHERE file_id = ?2",
                params![local, server],
            )?;
            tx.execute(
                "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(provisional_id) DO UPDATE SET server_id = excluded.server_id",
                params![local, server, input.now],
            )?;
            let successors = {
                let mut stmt = tx.prepare(&format!(
                    "SELECT {PENDING_OPERATION_COLUMNS} FROM operation_queue WHERE file_id = ?1 AND op_id != ?2 ORDER BY rowid"
                ))?;
                let rows = stmt.query_map(params![local, input.op_id], pending_operation_from_row)?;
                rows.collect::<Result<Vec<_>>>()?
            };
            for op in successors {
                match rekey(&op, server) {
                    Ok(metadata_json) => {
                        tx.execute(
                            "UPDATE operation_queue SET file_id = ?2, metadata_json = ?3, updated_at = ?4 WHERE op_id = ?1",
                            params![op.op_id, server, metadata_json, input.now],
                        )?;
                    }
                    // §8.6 rule 3: the chain step never fails the landing.
                    Err(_) => {
                        tx.execute(
                            "UPDATE operation_queue SET file_id = ?2, attempts = max_attempts, last_error = 'rekey_failed',
                                                        last_error_class = 'rekey_failed', updated_at = ?3
                             WHERE op_id = ?1",
                            params![op.op_id, server, input.now],
                        )?;
                        outcome.parked_successors.push((op.op_id, ParkReason::RekeyFailed));
                    }
                }
            }
            delete_file_conn(&tx, local)?;
        }
        if let Some(write_id) = input.write_id {
            // §8.6.2: only WHERE held_write_id = W; a later save's token stays held.
            tx.execute(
                "UPDATE files SET held_version = ?3, held_object_version_id = ?4 WHERE file_id = ?1 AND held_write_id = ?2",
                params![server, write_id, input.produced_version, input.produced_object_version_id],
            )?;
            // §8.1: the chain step, by write id.
            outcome.resolved_successors = tx.execute(
                "UPDATE operation_queue SET base_version = ?2, base_object_version_id = ?3, after_write_id = NULL
                 WHERE after_write_id = ?1",
                params![write_id, input.produced_version, input.produced_object_version_id],
            )?;
        } else {
            // Windows and watcher uploads keep round 3's equal-base rule (spec §6.3.1).
            outcome.resolved_successors = tx.execute(
                "UPDATE operation_queue SET base_version = ?3, base_object_version_id = ?4
                 WHERE file_id = ?1 AND op_id != ?2 AND write_id IS NULL
                   AND kind IN ('upload_version', 'upload_file')
                   AND ((?5 IS NOT NULL AND base_version = ?5) OR (?5 IS NULL AND ?6))",
                params![server, input.op_id, input.produced_version, input.produced_object_version_id, input.landed_base, local != server],
            )?;
        }
        // The op and its resume row; the payload is marked for release, unlinked after the commit (S6).
        let removed = match input.claim_id {
            Some(claim_id) => tx.execute(
                "DELETE FROM operation_queue WHERE op_id = ?1 AND claim_id = ?2",
                params![input.op_id, claim_id],
            )?,
            None => tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![input.op_id])?,
        };
        if input.claim_id.is_some() && removed != 1 {
            return Ok(None); // dropping `tx` rolls back
        }
        tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![input.op_id])?;
        if let Some(path) = input.release_payload {
            tx.execute(
                "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1) ON CONFLICT(path) DO UPDATE SET completed = 1",
                params![path],
            )?;
        }
        tx.commit()?;
        Ok(Some(outcome))
    }
```

(`default_contract(server)` is the `FileContractState` literal of `EB:1158-1177`, moved here as a private helper. `alias_target_for_test` and `set_target_path_for_test` are test-only one-line `SELECT`/`UPDATE`s.)

- [ ] **Step 4: The engine (`engine_bridge.rs`)**

4.1 `OpDone`:

```rust
/// What the runner still has to do after an op succeeded.
pub(crate) enum OpDone {
    /// Remove the op now (`finish_claimed`).
    Remove { release: Option<String> },
    /// The landing transaction already removed it; only unlink the released copy.
    Removed { release: Option<String> },
}
```

`execute_operation` returns `anyhow::Result<OpDone>`: non-upload arms `Ok(OpDone::Remove { release: None })`; the upload arm returns what `upload_version` returns. In `process_due_operations` the `Ok` arm calls `finish_claimed` only for `Remove`, then unlinks `release` for both.

4.2 `do_upload_version` (`EB:697`). Before the payload check (`EB:705`), a recorded completion lands without the network (§8.6 rule 4); it must come first, because a recorded completion never parks, not even on a missing payload (rule 6):

```rust
        if let Some(previous) = self.db.get_upload_resume(&op.op_id)?
            && let (Some(version), Some(object)) = (previous.completed_version, previous.completed_object_version_id.clone())
        {
            return self.land(local_file_id, op, claim, &previous.server_file_id, version, &object, previous.payload_size as u64, None, post_complete_errors, sync_root).await;
        }
```

In the `Ok(completed)` arm (`EB:815`), replace everything up to `Ok(released)` with:

```rust
            Ok(completed) => {
                // §8.6.1: an idempotent repeat answers `already_completed` with neither field
                // (UP:1299-1306); its version is base + 1 (1 for a create), its object id the session's.
                let produced_version = completed["version_number"]
                    .as_i64()
                    .unwrap_or_else(|| if is_create { 1 } else { op.base_version.unwrap_or(0).saturating_add(1) });
                let produced_object = completed["current_object_version_id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| session.object_version_id.clone());
                if let Some(claim) = claim
                    && !self.db.record_completion_claimed(&op.op_id, &claim.claim_id, produced_version, &produced_object)?
                {
                    log_queue_state_moved(&op.op_id, "completion");
                    return Err(anyhow::Error::new(QueueStateMoved));
                }
                let size = completed["size_bytes"].as_u64().unwrap_or(plaintext_size);
                self.land(local_file_id, op, claim, &session.server_file_id, produced_version, &produced_object, size, content_type, post_complete_errors, sync_root).await
            }
```

and add:

```rust
    #[allow(clippy::too_many_arguments)]
    async fn land(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        server_file_id: &str,
        produced_version: i64,
        produced_object: &str,
        size: u64,
        content_type: Option<String>,
        post_complete_errors: &mut Vec<String>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<OpDone> {
        self.seam("landing:after_complete");
        let payload_path = op.payload_path.clone();
        let release = if cfg!(target_os = "windows") { None } else { payload_path.clone() };
        let master_key = self.api.master_key();
        let landed = self.db.apply_landing(
            &crate::state_db::LandingInput {
                op_id: &op.op_id,
                claim_id: claim.map(|c| c.claim_id.as_str()),
                write_id: claim.and_then(|c| c.write.as_ref()).map(|w| w.write_id.as_str()),
                local_file_id,
                server_file_id,
                target_path: op.target_path.as_deref(),
                parent_id: op.parent_id.as_deref(),
                landed_base: op.base_version,
                produced_version,
                produced_object_version_id: produced_object,
                size_bytes: size as i64,
                content_type: content_type.as_deref(),
                release_payload: release.as_deref(),
                now: now_secs(),
            },
            &|queued, server_id| metadata_rekeyed_to(master_key, queued, server_id),
        )?;
        let Some(landed) = landed else {
            log_queue_state_moved(&op.op_id, "landing");
            return Err(anyhow::Error::new(QueueStateMoved));
        };
        for (successor, reason) in &landed.parked_successors {
            log_parked(successor, Some(server_file_id), *reason);
        }
        self.record_transfer_done(crate::transfer_progress::Direction::Up, server_file_id, size);
        if let Some(path) = payload_path.as_deref() {
            #[cfg(target_os = "windows")]
            self.defer_local_upload_finalization(op, server_file_id, sync_root, Path::new(path))?;
            let file_key = file_key_for(self.api.master_key(), server_file_id);
            if let Err(e) = self
                .finish_completed_upload(op, server_file_id, Path::new(path), content_type, &file_key, sync_root)
                .await
            {
                tracing::warn!(file_id = %server_file_id, error = %e, "upload-time thumbnail generation/upload skipped");
                post_complete_errors.push(format!("{}: {e}", op.op_id));
            }
        }
        Ok(if claim.is_some() { OpDone::Removed { release } } else { OpDone::Remove { release } })
    }
```

(Keep Mine passes `claim: None`: `apply_landing` removes nothing it does not own. `upload_version` now returns `anyhow::Result<OpDone>`, so Keep Mine's match from Task 2 step 5.7 becomes `Ok(OpDone::Remove { release } | OpDone::Removed { release })` and unlinks `release` itself.) Delete `apply_completed_upload`, `chain_queued_ops_after_upload` and Task 3's interim `carry_held_write` / `set_held_landed` / `resolve_successors` calls (and those three `StateDb` methods, now unused). Delete Task 6's `clear_version_filled` call in the landing (the transaction does it).

4.3 m-11, give-up. In `process_due_operations`' generic error arm, before computing `attempts`:

```rust
                        let completed = matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                            && self.db.get_upload_resume(&op.op_id)?.and_then(|r| r.completed_version).is_some();
                        // §8.6 rule 6: a recorded completion never parks and is never abandoned.
                        let attempts = if completed {
                            op.attempts.saturating_add(1).min(op.max_attempts.saturating_sub(1))
                        } else {
                            op.attempts.saturating_add(1)
                        };
                        if completed {
                            log_completed_landing_retried(&op.op_id, op.file_id.as_deref(), attempts);
                        }
```

and guard the give-up: `if attempts >= op.max_attempts && !completed { self.abandon_upload_after_give_up(&op).await; }`.

4.4 Mock: `delay_complete` applies to `POST /api/v1/uploads/{session}/complete` in `versioned_server_response`; `DELETE /api/v1/files/{id}` pushes `id` to `trashes` and answers `200 {"ok": true}`.

- [ ] **Step 5: GREEN, mutations**

Step-2 command into `$EVID/r4-t7-green.log`; expected `ok. 6 passed`. Mutations (`$EVID/r4-t7-mut-<T>.log`):
- T27: propagate the rekey error (`?` instead of the park arm) → the landing rolls back and the retry re-creates: two inits.
- T30: set `entry.status = Local` and the server's size at every landing → `local`.
- T40: split `apply_landing` into two transactions, the chain step in the first and the op removal in the second, with `self.seam("landing:after_complete")` between them → N is accepted after the chain step, waits on a write that is then removed, and parks `predecessor_lost`; the init summary has one entry.
- T61: abandon at give-up regardless of `completed` → a trash request.
- P9: drop the recorded-completion shortcut → the retry re-runs the session (requests grow).
- P10: drop the `id_aliases` insert → `alias_target_for_test` is `None`.

- [ ] **Step 6: Gate and commit**

Task gate; expected lib count **`L0 + 50`**. Every Keep Mine, Windows-finalization and round-3 chain test stays green.

```bash
cd $WT && git commit -m "feat(desktop): land an upload in one transaction after recording the server's completion" -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs
```

**Stop point:** after the commit.

---

## Task 8: Rule 3: provisional ids, fetches from the queue, thumbnails, unknown ids (spec §7, §5.6 thumbnail safe default; commit 8)

After a create lands, every request under its provisional id reaches the server file. A fetch of an item whose write is queued is served from the queued bytes. An unknown id is refused, never answered "no item".

**Files:**
- Modify: `src-tauri/src/state_db.rs`: `resolve_alias`, `sweep_aliases`, `queue_fetch_source`; `engine_start_repair` sweeps aliases older than 30 days
- Modify: `src-tauri/src/engine_bridge.rs`: `FpWrite.present_as`; `UnknownItem`, `ThumbnailNotYet`; `resolve_provisional`; `queue_file_provider_modify_from` resolves and refuses (replacing Task 3's interim `UnknownItem` enqueue); `queue_file_provider_deletion_conflicted_create`; `queue_finder_delete` (`EB:1741-1789`) resolves through the alias only with the held token as base; `HydrateSource`, `serve_hydrate`; `hydrate_file_with_progress` (`EB:2557-2690`) leaves a row with a live upload alone; `write_hydrated_plaintext` (`EB:4411-4518`, `EB:4521-4524`) gains a reader-based core; `finder_thumbnail`; the log helpers; the mock gains thumbnail and file-metadata routes
- Modify: `src-tauri/src/ipc_socket.rs`: `QueueFinderCreate` (`IPC:54-66`) gains three optional fields and its arm dispatches; `write_outcome_response` (`IPC:2364-2396`) becomes `pub(crate)`, refuses an unreadable queued row, presents `present_as`; `write_refusal_category` (`IPC:1412-1431`) maps `UnknownItem`; `file_status_response` extracted from the `GetFileStatus` arm (`IPC:1760-1765`) resolves the alias; `hydrate_over_ipc` (`IPC:2128`) calls `serve_hydrate` and logs `Finder hydrate served`; the `FetchThumbnail` arm (`IPC:1921-1966`) calls `finder_thumbnail`
- Modify: `src-tauri/src/runner.rs`: a daily `sweep_aliases`
- Modify: `BeebeebFileProvider/IPCFraming.swift` (`IPCWriteRequest.create`, `:573-603`), `BeebeebFileProvider/XPCBridge.swift` (`queueCreateItem`, `:396-427`), `BeebeebFileProvider/FileProviderExtension.swift` (`QueuedWriteCompletion` `:388-391`, `queuedWriteCompletion` `:412-420`, `createItem` `:445-483`, `modifyItem`'s completions)
- Modify: `BeebeebFileProviderTests/main.swift` (T20, T44; the no-item lines of the check at `:1767-1784` move into T20); `scripts/test-ipc-framing.sh:20` `EXPECTED_TESTS=96`
- Test: `engine_bridge.rs` tests

**Interfaces:**
- Consumes: aliases and held columns written by the landing (Task 7); `accept_finder_write` (Task 3); `item_presentation` (Task 1).
- Produces:
  - `StateDb::resolve_alias(&self, id: &str) -> Result<Option<String>>` (only when no live row has `id`); `StateDb::sweep_aliases(&self, now: i64, max_age_secs: i64) -> Result<usize>`; `StateDb::queue_fetch_source(&self, file_id: &str) -> Result<Option<String>>` (S1.8: the held write's payload path when that write is queued, one read); `pub const ALIAS_MAX_AGE_SECS: i64 = 30 * 86_400`
  - `FpWrite { outcome, token, present_as: Option<String> }` (Task 3's literals gain `present_as: None`)
  - `EngineBridge::resolve_provisional(&self, id: &str, request: &'static str) -> anyhow::Result<(String, Option<String>)>` (the id to act on, and the id to present the reply under)
  - `EngineBridge::queue_file_provider_deletion_conflicted_create(&self, target: FinderWriteTarget, template_identifier: Option<String>, template_content_version: Option<String>, contents: Option<&std::fs::File>) -> anyhow::Result<FpWrite>`
  - `pub enum HydrateSource { Queue, Server }` (`as_str`: `"queue"`, `"server"`); `EngineBridge::serve_hydrate(&self, id: &str, dest: &Path, allowed_roots: &[&Path], progress: Option<&HydrateProgressFn>) -> anyhow::Result<HydrateSource>`
  - `EngineBridge::finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>>`
  - `pub(crate) fn write_hydrated_from_reader(dest_path: &Path, allowed_roots: &[&Path], reader: &mut dyn std::io::Read) -> std::io::Result<u64>`
  - `ipc_socket::write_outcome_response` and `ipc_socket::file_status_response(db: &StateDb, file_id: &str) -> IpcResponse` are `pub(crate)`
  - log helpers: `log_alias_resolved(request: &'static str, provisional_id: &str, file_id: &str)` ("provisional id resolved to the server id"), `log_alias_delete_kept(provisional_id: &str, file_id: &str)` ("delete of a provisional id not applied to the server file")
  - Swift: `IPCWriteRequest.create(…, contents:, deletionConflicted: Bool = false, templateIdentifier: String? = nil, templateContentVersion: String? = nil)`; `XPCBridge.queueCreateItem(…, contentType:, deletionConflicted: Bool = false, templateIdentifier: String? = nil, templateContentVersion: String? = nil)`; `QueuedWriteCompletion.error: Error?`
  - mock: `GET /api/v1/files/{id}/thumbnail/{variant}` answers `s.thumbnails[id]` (an encrypted blob the test seeds) or 404, and records `thumbnail_requests: Vec<String>` (the file id); `GET /api/v1/files/{id}` answers 404 (no metadata route: a server fetch fails)

- [ ] **Step 1: Write the failing tests**

Swift, in `BeebeebFileProviderTests/main.swift`: delete the `noItem` lines from the check at `:1773-1777` (they move here) and add:

```swift
check("a queued write without an item is an error, never a nil item") {
    let noItem = FileProviderExtension.queuedWriteCompletion(
        WriteQueueResult(item: nil, ignored: false, message: "queued")
    )
    try expect(noItem.item == nil, "no item from the app means no item for the system")
    guard let error = noItem.error as? BeebeebIPCError else {
        throw TestFailure(description: "a queued write without an item must complete with an error")
    }
    try expect((error as NSError).code == NSFileProviderError.serverUnreachable.rawValue,
               "serverUnreachable: the system retries and keeps the file on disk (REPL.h:731-736)")
    try expect(error.isTransient, "transient, never definitive")
    let ignored = FileProviderExtension.queuedWriteCompletion(
        WriteQueueResult(item: nil, ignored: true, message: "ignored temporary item")
    )
    try expect(ignored.error == nil, "an ignored temporary item is not an error")
}

check("createItem passes deletionConflicted, the template id and its content version") {
    let token = "0:w" + String(repeating: "a", count: 32)
    let conflicted = IPCWriteRequest.create(
        parentIdentifier: "NSFileProviderRootContainerItemIdentifier", filename: "t2.txt", kind: "file",
        contentsPath: "/tmp/staged", contentType: "public.plain-text", contents: sampleContents,
        deletionConflicted: true, templateIdentifier: "3f2a9c1e-0000-4000-8000-000000000009",
        templateContentVersion: token
    )
    let payload = conflicted["QueueFinderCreate"] as? [String: Any] ?? [:]
    try expect(payload["deletion_conflicted"] as? Bool == true, "the option is passed")
    try expect(payload["template_identifier"] as? String == "3f2a9c1e-0000-4000-8000-000000000009", "the template id")
    try expect(payload["template_content_version"] as? String == token, "the template's content version")
    let plain = sampleCreateRequest()["QueueFinderCreate"] as? [String: Any] ?? [:]
    try expect(plain["deletion_conflicted"] as? Bool == false, "without the option: false")
    try expect(plain["template_identifier"] == nil && plain["template_content_version"] == nil, "no template fields")
}
```

Rust, in `engine_bridge.rs` tests (setup as in Task 3):

```rust
    /// A create landed: returns (P, S, the create's token).
    async fn landed_create(bridge: &EngineBridge, server: &VersionedServerMock, dir: &Path, sync_root: &Path, name: &str) -> (String, String, String) {
        let created = fp_create(bridge, dir, name, b"created bytes");
        let provisional = created.outcome_file_id();
        drain_upload_queue(bridge, sync_root).await;
        let server_id = server.state.lock().unwrap().files_with_content()[0].clone();
        (provisional, server_id, created.token.unwrap())
    }

    #[tokio::test]
    async fn i3_create_lands_then_a_modify_under_the_provisional_id() {
        // master_key [85u8; 32]
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t.txt").await;
        let reply = fp_save(&bridge, dir.path(), &p, "t.txt", b"created bytes, edited", &create_token);
        assert_eq!(reply.present_as.as_deref(), Some(p.as_str()));
        let ipc = crate::ipc_socket::write_outcome_response("modify", &bridge.db, Ok(reply.clone()));
        let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else { panic!("an item, never None: {ipc:?}") };
        assert_eq!(item.identifier, p, "presented under P until the system applies the swap");
        assert_eq!(item.content_version, reply.token, "the new token");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files_with_content(), vec![s.clone()], "one server file");
        assert_eq!(state.files[&s].versions.len(), 2);
        assert_eq!(state.latest_plaintext(&s, master_key), b"created bytes, edited");
    }

    #[tokio::test]
    async fn a_delete_of_the_provisional_id_after_landing_trashes_the_server_file() {
        // master_key [86u8; 32]
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "gone.txt").await;
        bridge.queue_finder_delete(&p, Some(create_token)).unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(server.finish().trashes, vec![s]);
    }

    #[tokio::test]
    async fn a_delete_through_the_alias_needs_the_held_token_as_base() {
        // master_key [87u8; 32]; m-6
        let (p, s, _create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "kept.txt").await;
        let logs = capture_logs_async(async {
            bridge.queue_finder_delete(&p, Some("1".into())).unwrap();
            drain_upload_queue(&bridge, &sync_root).await;
        })
        .await;
        assert!(server.state.lock().unwrap().trashes.is_empty(), "S kept");
        assert!(logs.contains("delete of a provisional id not applied to the server file"), "{logs}");
        assert!(bridge.db.get_file(&s).unwrap().is_some());
        drop(server.finish());
    }

    #[tokio::test]
    async fn an_unknown_id_is_refused_never_answered_without_an_item() {
        // master_key [88u8; 32]
        let contents = dir.path().join("orphan.txt");
        std::fs::write(&contents, b"edit of an item nobody knows").unwrap();
        let unknown = "3f2a9c1e-0000-4000-8000-00000000dead";
        let logs = capture_logs_async(async {
            let result = bridge.queue_file_provider_modify(finder_file_target(Some(unknown), "orphan.txt", &contents, Some("1".into())));
            let reply = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
            assert!(matches!(reply, crate::ipc_socket::IpcResponse::Error { .. }), "{reply:?}");
        })
        .await;
        assert!(bridge.db.list_due_operations(i64::MAX).unwrap().is_empty(), "nothing queued");
        assert!(logs.contains("Finder write refused") && logs.contains("unknown_item"), "{logs}");
        drop(server.finish());
    }

    #[tokio::test]
    async fn r2a_a_fetch_during_the_creates_queue_wait_is_served_locally() {
        // master_key [89u8; 32]; the mock answers 404 to anything unexpected
        let created = fp_create(&bridge, dir.path(), "r2.txt", b"eight million bytes, in spirit");
        let p = created.outcome_file_id();
        let dest = dir.path().join("fetch").join("r2.txt");
        let source = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await.unwrap();
        assert_eq!(source, HydrateSource::Queue);
        assert_eq!(std::fs::read(&dest).unwrap(), b"eight million bytes, in spirit", "the staged bytes");
        assert_eq!(bridge.db.get_file(&p).unwrap().unwrap().status, FileStatus::Uploading, "still writable");
        assert!(server.finish().requests.is_empty(), "nothing reached the server");
    }

    #[tokio::test]
    async fn a_failed_hydrate_never_changes_a_row_with_a_live_upload() {
        // master_key [90u8; 32]
        seed_uploaded_row(&bridge, &server, "live-row");
        fp_save(&bridge, dir.path(), "live-row", "notes.txt", b"queued edit", "1");
        let op = bridge.db.list_operations_for_file("live-row").unwrap().remove(0);
        std::fs::remove_file(op.payload_path.as_deref().unwrap()).unwrap();
        let dest = dir.path().join("fetch").join("notes.txt");
        assert!(bridge.serve_hydrate("live-row", &dest, &[dir.path()], None).await.is_err(), "the server has no metadata route");
        assert_eq!(bridge.db.get_file("live-row").unwrap().unwrap().status, FileStatus::Uploading);
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_deletion_conflicted_create_of_the_provisional_id_modifies_the_server_file() {
        // master_key [91u8; 32]; I-1
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "t2.txt").await;
        let edited = dir.path().join("t2-edited.txt");
        std::fs::write(&edited, b"created bytes, edited before the swap").unwrap();
        let reply = bridge
            .queue_file_provider_deletion_conflicted_create(finder_file_target(None, "t2.txt", &edited, None), Some(p.clone()), Some(create_token), None)
            .unwrap();
        let crate::engine_bridge::FinderWriteOutcome::Queued { file_id: Some(id), .. } = &reply.outcome else { panic!("{reply:?}") };
        assert_eq!(id, &s, "a content modify of S");
        assert!(reply.present_as.is_none(), "a create reply names the provider's identifier, S (REPL.h:438-443)");
        // Guard: without an alias it is an ordinary create.
        let other = dir.path().join("t3.txt");
        std::fs::write(&other, b"a file deleted elsewhere, edited here").unwrap();
        bridge
            .queue_file_provider_deletion_conflicted_create(finder_file_target(None, "t3.txt", &other, None), Some("3f2a9c1e-0000-4000-8000-0000000000aa".into()), Some("0".into()), None)
            .unwrap();
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files[&s].versions.len(), 2, "one new version of S");
        assert_eq!(state.latest_plaintext(&s, master_key), b"created bytes, edited before the swap");
        assert_eq!(state.files_with_content().len(), 2, "S and the guard's new file; no second t2");
    }

    #[tokio::test]
    async fn the_alias_resolves_rename_move_item_thumbnail_and_hydrate() {
        // master_key [92u8; 32]; one case per request kind
        let (p, s, create_token) = landed_create(&bridge, &server, dir.path(), &sync_root, "aliased.txt").await;
        let blob = beebeeb_core::encrypt::encrypt_chunk_raw(
            &beebeeb_core::kdf::derive_file_key(&beebeeb_core::kdf::MasterKey::from_bytes(master_key), s.as_bytes()),
            b"thumbnail bytes",
        )
        .unwrap();
        server.state.lock().unwrap().thumbnails.insert(s.clone(), blob);
        let rename = |parent: Option<&str>, name: &str| FinderWriteTarget {
            file_id: Some(p.clone()), parent_id: parent.map(str::to_string), filename: name.into(), rel_path: None,
            kind: FinderWriteItemKind::File, contents_path: None, content_type: None, base_version_identifier: Some(create_token.clone()),
        };
        // rename
        bridge.queue_file_provider_modify(rename(None, "renamed.txt")).unwrap();
        // move
        bridge.queue_file_provider_modify(rename(Some("3f2a9c1e-0000-4000-8000-0000000000f0"), "renamed.txt")).unwrap();
        let ops = bridge.db.list_operations_for_file(&s).unwrap();
        assert_eq!(ops.iter().map(|op| op.kind.clone()).collect::<Vec<_>>(), vec![OperationKind::RenameFile, OperationKind::MoveFile], "rename and move reach S");
        // item
        let crate::ipc_socket::IpcResponse::FileStatus(item) = crate::ipc_socket::file_status_response(&bridge.db, &p) else { panic!("item under P") };
        assert_eq!(item.identifier, p);
        // thumbnail
        assert_eq!(&bridge.finder_thumbnail(&p, "small").await.unwrap()[..], b"thumbnail bytes");
        // hydrate (no write queued: the server is asked for S)
        let dest = dir.path().join("fetch").join("aliased.txt");
        let _ = bridge.serve_hydrate(&p, &dest, &[dir.path()], None).await;
        let state = server.finish();
        assert_eq!(state.thumbnail_requests, vec![s.clone()]);
        assert!(state.requests.iter().any(|(m, path)| m == "GET" && path == &format!("/api/v1/files/{s}")), "the hydrate asked for S: {:?}", state.requests);
    }

    #[test]
    fn aliases_older_than_30_days_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let now = now_secs();
        db.insert_alias_for_test("young", "s1", now - 29 * 86_400);
        db.insert_alias_for_test("old", "s2", now - 31 * 86_400);
        db.engine_start_repair().unwrap(); // at start
        assert_eq!(db.resolve_alias("young").unwrap(), Some("s1".into()));
        assert_eq!(db.resolve_alias("old").unwrap(), None);
        db.insert_alias_for_test("old-again", "s3", now - 31 * 86_400);
        assert_eq!(db.sweep_aliases(now, crate::state_db::ALIAS_MAX_AGE_SECS).unwrap(), 1); // daily
    }

    #[tokio::test]
    async fn the_thumbnail_of_a_queued_write_is_an_error_without_a_server_request() {
        // master_key [93u8; 32]; the safe default (spec §5.6, T56′)
        seed_uploaded_row(&bridge, &server, "photo");
        let blob = beebeeb_core::encrypt::encrypt_chunk_raw(
            &beebeeb_core::kdf::derive_file_key(&beebeeb_core::kdf::MasterKey::from_bytes(master_key), b"photo"),
            b"new thumbnail",
        )
        .unwrap();
        server.state.lock().unwrap().thumbnails.insert("photo".into(), blob);
        fp_save(&bridge, dir.path(), "photo", "photo.png", b"new image bytes", "1");
        assert!(bridge.finder_thumbnail("photo", "small").await.is_err(), "an error while the write is queued");
        assert!(server.state.lock().unwrap().thumbnail_requests.is_empty(), "0 requests: never the old version's thumbnail");
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(&bridge.finder_thumbnail("photo", "small").await.unwrap()[..], b"new thumbnail", "served after the landing");
        assert_eq!(server.finish().thumbnail_requests, vec!["photo".to_string()]);
    }

    #[tokio::test]
    async fn an_empty_save_lands_and_is_served_from_the_queue() {
        // master_key [94u8; 32]; Review Focus 3
        seed_uploaded_row(&bridge, &server, "empty");
        fp_save(&bridge, dir.path(), "empty", "notes.txt", b"", "1");
        let dest = dir.path().join("fetch").join("notes.txt");
        assert_eq!(bridge.serve_hydrate("empty", &dest, &[dir.path()], None).await.unwrap(), HydrateSource::Queue);
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), 0, "an empty file, not an error");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files["empty"].versions.len(), 2);
        assert!(state.latest_plaintext("empty", master_key).is_empty());
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- i3_create_lands_then a_delete_of_the_provisional_id a_delete_through_the_alias an_unknown_id_is_refused r2a_a_fetch_during a_failed_hydrate_never a_deletion_conflicted_create the_alias_resolves aliases_older_than the_thumbnail_of_a_queued an_empty_save_lands > $EVID/r4-t8-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t8-red.log | head -20
cd $WT && bash scripts/test-ipc-framing.sh > $EVID/r4-t8-swift-red.log 2>&1; echo "swift rc=$?"; tail -3 $EVID/r4-t8-swift-red.log
```

Expected: Rust compile errors for the new items, then the spec's REDs: T15 a second server file and a reply with no item; T16 an idempotent success with S kept; T17 `WriteQueued` with an op queued; T18 a 404 from the server and the row `Error`; T19 `Error`; T43 a second server file; T54 P unknown; T56′ the server's old thumbnail fetched; T58 is new (its first case is T16's); RF3 a 404. Swift: does not compile until the new parameters exist; then T20 RED on "must complete with an error".

- [ ] **Step 3: `state_db.rs`**

```rust
pub const ALIAS_MAX_AGE_SECS: i64 = 30 * 86_400;

    /// §7.1–§7.2: the server id a provisional id became, while no live row has it.
    pub fn resolve_alias(&self, id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT server_id FROM id_aliases
             WHERE provisional_id = ?1 AND NOT EXISTS (SELECT 1 FROM files WHERE file_id = ?1)",
            params![id],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn sweep_aliases(&self, now: i64, max_age_secs: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM id_aliases WHERE created_at < ?1", params![now - max_age_secs])
    }

    /// §7.4 and S1.8: the held write's staged copy, when that write is still queued.
    pub fn queue_fetch_source(&self, file_id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT q.payload_path FROM files f
             JOIN operation_queue q ON q.write_id = f.held_write_id
             WHERE f.file_id = ?1 AND q.payload_path IS NOT NULL",
            params![file_id],
            |row| row.get(0),
        )
        .optional()
    }
```

`engine_start_repair` adds `tx.execute("DELETE FROM id_aliases WHERE created_at < ?1", params![now_secs() - ALIAS_MAX_AGE_SECS])?;` (`now_secs`, `SD:350`).

- [ ] **Step 4: The engine (`engine_bridge.rs`)**

4.1 Errors and log lines:

```rust
/// A Finder write for an id this Mac never knew and no alias resolves (spec §7.3).
#[derive(Debug)]
pub(crate) struct UnknownItem;
impl std::fmt::Display for UnknownItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this item is not known to Beebeeb")
    }
}
impl std::error::Error for UnknownItem {}

/// §5.6 safe default: no thumbnail is fetched while the write is queued.
#[derive(Debug)]
pub(crate) struct ThumbnailNotYet;
impl std::fmt::Display for ThumbnailNotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the thumbnail is available once the save has uploaded")
    }
}
impl std::error::Error for ThumbnailNotYet {}

fn log_alias_resolved(request: &'static str, provisional_id: &str, file_id: &str) {
    tracing::warn!(request, provisional_id = %provisional_id, file_id = %file_id, "provisional id resolved to the server id");
}

fn log_alias_delete_kept(provisional_id: &str, file_id: &str) {
    tracing::warn!(provisional_id = %provisional_id, file_id = %file_id, "delete of a provisional id not applied to the server file");
}
```

4.2 `resolve_provisional`:

```rust
    pub fn resolve_provisional(&self, id: &str, request: &'static str) -> anyhow::Result<(String, Option<String>)> {
        if self.db.get_file(id)?.is_some() {
            return Ok((id.to_string(), None));
        }
        match self.db.resolve_alias(id)? {
            Some(server_id) => {
                log_alias_resolved(request, id, &server_id);
                Ok((server_id, Some(id.to_string())))
            }
            None => Ok((id.to_string(), None)),
        }
    }
```

4.3 `queue_file_provider_modify_from`: first `let (file_id, present_as) = self.resolve_provisional(&requested_id, "modify")?;` and `target.file_id = Some(file_id.clone())`. If `self.db.get_file(&file_id)?` is `None` (no row, no alias): `return Err(anyhow::Error::new(UnknownItem))` for content and metadata modifies alike, before anything is staged (§7.3). Task 3's interim `UnknownItem` enqueue is deleted; `AcceptOutcome::UnknownItem` now maps to the same error. Every returned `FpWrite` carries `present_as`.

4.4 `queue_file_provider_deletion_conflicted_create`:

```rust
    pub fn queue_file_provider_deletion_conflicted_create(
        &self,
        target: FinderWriteTarget,
        template_identifier: Option<String>,
        template_content_version: Option<String>,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        // §7.2 (I-1): resolved through the alias, or a live row with that id.
        let resolved = match template_identifier.as_deref() {
            Some(id) => match self.db.get_file(id)? {
                Some(row) => Some(row),
                None => match self.db.resolve_alias(id)? {
                    Some(server_id) => {
                        log_alias_resolved("deletion_conflicted_create", id, &server_id);
                        self.db.get_file(&server_id)?
                    }
                    None => None,
                },
            },
            None => None,
        };
        match resolved {
            // A content modify of S, replied under S (REPL.h:438-443): the reuse rule replaces item S on disk.
            Some(row) if row.status != FileStatus::Trashing => self.queue_file_provider_modify_from(
                FinderWriteTarget {
                    file_id: Some(row.file_id),
                    parent_id: target.parent_id,
                    filename: target.filename,
                    rel_path: None,
                    kind: FinderWriteItemKind::File,
                    contents_path: target.contents_path,
                    content_type: target.content_type,
                    base_version_identifier: template_content_version,
                },
                contents,
            ),
            // No alias and no row (a file deleted elsewhere), or a row going to the trash: an ordinary create.
            _ => self.queue_file_provider_create_from(target, contents),
        }
    }
```

4.5 `queue_finder_delete` (`EB:1741`). Before the "unknown item" branch (`EB:1754-1761`), when `self.db.get_file(file_id)?` is `None` and `self.db.resolve_alias(file_id)?` is `Some(server_id)`:

```rust
            let held = self.db.item_presentation(&server_id)?.and_then(|p| p.held).map(|h| h.token());
            if held.is_some() && held == base_version_identifier {
                log_alias_resolved("delete", file_id, &server_id);
                return self.queue_finder_delete(&server_id, base_version_identifier); // trashes S
            }
            log_alias_delete_kept(file_id, &server_id);
            return Ok(FinderWriteOutcome::Ignored { message: "the item is already gone from Beebeeb".to_string() });
```

4.6 The reader-based hydrate write. Split `write_hydrated_plaintext` (`EB:4411-4518`) into `write_hydrated_from_reader(dest_path, allowed_roots, reader: &mut dyn std::io::Read) -> std::io::Result<u64>`: every step stays (containment walk, the `O_NOFOLLOW` temp file, the atomic rename); only the write at `EB:4501` becomes `std::io::copy(reader, &mut file)`, returning the byte count. `write_hydrated_plaintext` becomes `write_hydrated_from_reader(dest_path, allowed_roots, &mut &buf[..]).map(|_| ())`. The non-unix twin (`EB:4521-4524`) copies with `std::io::copy` into `File::create`.

4.7 `serve_hydrate`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrateSource {
    Queue,
    Server,
}

impl HydrateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HydrateSource::Queue => "queue",
            HydrateSource::Server => "server",
        }
    }
}

    /// §7.4: a fetch of an item whose held write is queued is served from that write's
    /// staged bytes, the bytes its token names; otherwise from the server, as today.
    pub async fn serve_hydrate(
        &self,
        id: &str,
        dest: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<HydrateSource> {
        if !hydrate_dest_is_allowed(dest, allowed_roots) {
            anyhow::bail!("hydrate destination is not within an allowed root");
        }
        let (file_id, _) = self.resolve_provisional(id, "hydrate")?;
        // Decide once more when the landing unlinked the copy between the read and the open.
        for _ in 0..2 {
            let Some(path) = self.db.queue_fetch_source(&file_id)? else { break };
            match std::fs::File::open(&path) {
                Ok(mut staged) => {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = write_hydrated_from_reader(dest, allowed_roots, &mut staged)?;
                    if let Some(progress) = progress {
                        progress(bytes, bytes);
                    }
                    log_hydrate_served(&file_id, HydrateSource::Queue);
                    return Ok(HydrateSource::Queue);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            }
        }
        self.hydrate_file_with_progress(&file_id, dest, allowed_roots, progress).await?;
        log_hydrate_served(&file_id, HydrateSource::Server);
        Ok(HydrateSource::Server)
    }

/// The device plan's count of fetches that reached the daemon (spec §11, §14 instrument 2).
fn log_hydrate_served(file_id: &str, source: HydrateSource) {
    tracing::info!(file_id = %file_id, source = source.as_str(), "Finder hydrate served");
}
```

4.8 `hydrate_file_with_progress` (§7.4.2): after the path checks (`EB:2576`), `let touch_status = !self.db.item_presentation(file_id)?.is_some_and(|p| p.unparked_finder_upload);`. Arm the `DownloadingStatusGuard` and call each `set_status` (`EB:2583-2584`, `:2609`, `:2633`, `:2647`, `:2681`) only when `touch_status`.

4.9 `finder_thumbnail`:

```rust
    pub async fn finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (file_id, _) = self.resolve_provisional(id, "thumbnail")?;
        if self.db.item_presentation(&file_id)?.is_some_and(|p| p.held_write_queued) {
            return Err(anyhow::Error::new(ThumbnailNotYet));
        }
        self.fetch_thumbnail_to_memory(&file_id, variant).await
    }
```

4.10 Mock: `thumbnails: HashMap<String, Vec<u8>>` and `thumbnail_requests: Vec<String>`; `GET /api/v1/files/{id}/thumbnail/{variant}` records `id` and answers the blob (`200`, `application/octet-stream`) or 404. `http_json` serves JSON; add a small `http_bytes(status, body)` next to it (`EB:6667-6700` region of the test helpers).

- [ ] **Step 5: `ipc_socket.rs`**

5.1 `QueueFinderCreate` gains `#[serde(default)] deletion_conflicted: bool, #[serde(default)] template_identifier: Option<String>, #[serde(default)] template_content_version: Option<String>`. In its arm, inside the `dedup_write` closure: `if deletion_conflicted { work_bridge.queue_file_provider_deletion_conflicted_create(target, template_identifier, template_content_version, opened) } else { work_bridge.queue_file_provider_create_from(target, opened) }`. An older extension sends none of them and gets today's create.

5.2 `write_outcome_response` becomes `pub(crate)`. For `Queued { file_id: Some(id), .. }`: read the row; if it is missing, `log_refused_write(op, "row_unreadable")` and answer `IpcResponse::Error { message: "the write was queued but its item could not be read; try again".into() }` (§7.3: no item-less reply unless ignored). Otherwise build the payload and, when `present_as` is `Some(p)`, set `item.identifier = p`.

5.3 `write_refusal_category` (`IPC:1412`): first `if error.downcast_ref::<crate::engine_bridge::UnknownItem>().is_some() { return "unknown_item"; }`.

5.4 Extract the `GetFileStatus` arm into `pub(crate) fn file_status_response(db: &StateDb, file_id: &str) -> IpcResponse`: a live row as today; otherwise `db.resolve_alias(file_id)` → `log_alias_resolved` (make that helper `pub(crate)`), the payload of S with `identifier = file_id`; otherwise `Error { "not found" }`.

5.5 `hydrate_over_ipc` (`IPC:2128`): `bridge.serve_hydrate(file_id, dest, &allowed_roots, progress_cb)`; `Ok(_)` answers `IpcResponse::Ok {}`. The `Finder hydrate served` line is logged inside `serve_hydrate` (4.7), so engine tests can capture it.

5.6 The `FetchThumbnail` arm (`IPC:1949-1951`): `bridge.finder_thumbnail(&file_id, thumbnail_variant(max_dimension)).await`. Its error already becomes `IpcResponse::Error`, which the extension turns into a per-item error (`XPC:534-535`, `FPE:1019`): no Swift change.

- [ ] **Step 6: Swift**

6.1 `IPCWriteRequest.create` (`IPCFraming.swift:573`): three parameters with defaults after `contents:`; in the body, `payload["deletion_conflicted"] = deletionConflicted`, and when it is true, `payload["template_identifier"] = templateIdentifier` and `payload["template_content_version"] = templateContentVersion` (only non-nil values). The idempotency key is unchanged.

6.2 `XPCBridge.queueCreateItem` (`XPCBridge.swift:396`): the same three parameters with defaults, passed to `IPCWriteRequest.create` on new lines after the unchanged `contents: contentsURL.flatMap { IPCContentFingerprint.ofFile(at: $0) }` line, so `scripts/check-ipc-timeouts.py` still finds its call-site text.

6.3 `FileProviderExtension.swift`:

```swift
    struct QueuedWriteCompletion {
        let item: NSFileProviderItem?
        let shouldFetchContent: Bool
        let error: Error?
    }

    static func queuedWriteCompletion(_ result: WriteQueueResult) -> QueuedWriteCompletion {
        if result.ignored {
            return QueuedWriteCompletion(item: nil, shouldFetchContent: false, error: nil)
        }
        if let model = result.item {
            return QueuedWriteCompletion(item: FileProviderItem(model: model), shouldFetchContent: false, error: nil)
        }
        // A nil item would make the system delete the item on disk (REPL.h:629-634).
        return QueuedWriteCompletion(
            item: nil,
            shouldFetchContent: false,
            error: BeebeebIPCError.invalidResponse("the app queued the write but returned no item")
        )
    }
```

Update the doc comment's third bullet to say so. In `createItem` (`:445-483`):

```swift
            let deletionConflicted = options.contains(.deletionConflicted)
            let result = try ipc.queueCreateItem(
                parentIdentifier: itemTemplate.parentItemIdentifier,
                filename: itemTemplate.filename,
                kind: kind,
                contentsURL: url,
                contentType: kind == .file ? contentType?.identifier : nil,
                deletionConflicted: deletionConflicted,
                templateIdentifier: deletionConflicted ? itemTemplate.itemIdentifier.rawValue : nil,
                templateContentVersion: deletionConflicted ? itemTemplate.itemVersion.flatMap { Self.versionIdentifier($0) } : nil
            )
            let completion = Self.queuedWriteCompletion(result)
            completionHandler(completion.item, [], completion.shouldFetchContent, completion.error)
            if completion.error == nil {
                signalErrorResolvedIfPending()
            }
```

and every other `queuedWriteCompletion` call site in `modifyItem` passes `completion.error` the same way. (`itemVersion` is an optional requirement of `NSFileProviderItem`, `ITEM.h:355`, `:590`, so it reads as an optional here.)

6.4 `scripts/test-ipc-framing.sh:20`: `EXPECTED_TESTS=96`.

- [ ] **Step 7: The daily sweep (`runner.rs`)**

In the tick loop (next to the other periodic work before `sync_tick_outcome`, `RUN:1293`), keep `let mut last_alias_sweep = std::time::Instant::now();` outside the loop and, inside it, when `last_alias_sweep.elapsed() >= Duration::from_secs(86_400)`, call `db.sweep_aliases(now_secs(), crate::state_db::ALIAS_MAX_AGE_SECS)` (a warn on error) and reset the instant. Engine start sweeps through `engine_start_repair`.

- [ ] **Step 8: GREEN, mutations**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- i3_create_lands_then a_delete_of_the_provisional_id a_delete_through_the_alias an_unknown_id_is_refused r2a_a_fetch_during a_failed_hydrate_never a_deletion_conflicted_create the_alias_resolves aliases_older_than the_thumbnail_of_a_queued an_empty_save_lands > $EVID/r4-t8-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/r4-t8-green.log
cd $WT && bash scripts/test-ipc-framing.sh > $EVID/r4-t8-swift-green.log 2>&1; echo "swift rc=$?"; tail -1 $EVID/r4-t8-swift-green.log
bash scripts/test-ipc-framing.sh --self-test > $EVID/r4-t8-swift-selftest.log 2>&1; echo "self-test rc=$?"
python3 scripts/check-ipc-timeouts.py > $EVID/r4-t8-ipc-timeouts.log 2>&1; echo "guard rc=$?"; python3 scripts/check-ipc-timeouts.py --self-test >> $EVID/r4-t8-ipc-timeouts.log 2>&1; echo "guard self-test rc=$?"
bash scripts/build-fileprovider-extension.sh --out "$WT/src-tauri/target/fp-check/BeebeebFileProvider.appex" --helper-out "$WT/src-tauri/target/fp-check/BeebeebFileProviderCtl" > $EVID/r4-t8-extension-build.log 2>&1; echo "extension rc=$?"
```

Expected: `ok. 11 passed`; Swift `ipc-framing: 96 passed, 0 failed`; both self-tests and the guard `rc=0`; the extension builds (`rc=0`, the CI recipe `ci.yml:215-225`, unsigned). Mutations (`$EVID/r4-t8-mut-<T>.log`):
- T15: skip the alias lookup in `queue_file_provider_modify_from` → a second server file.
- T16 / T58: drop the alias branch in `queue_finder_delete` (T16 red) / drop its held-token gate (T58 red: S trashed).
- T17: answer `FinderWriteOutcome::Ignored` for an unknown id → `WriteQueued`.
- T18: skip the queue in `serve_hydrate` → a server request and `Error`.
- T19: keep today's `set_status` calls → `Error`.
- T43: make `queue_file_provider_deletion_conflicted_create` always call `queue_file_provider_create_from` (the option ignored, today's behaviour) → a second `t2` file; "a content modify of S" fails.
- T54: skip the alias for one kind at a time (five runs, each red on its own case).
- T55: drop the sweep from `engine_start_repair` → "old" still resolves.
- T56′: drop the `held_write_queued` check → the server is asked while queued.
- RF3: treat a 0-byte staged copy as missing (`metadata().len() == 0` → `NotFound`) → the hydrate goes to the server and fails.
- T20 (Swift): restore the nil branch (`error: nil`) → "must complete with an error" fails (`$EVID/r4-t8-swift-mut-T20.log`).
- T44 (Swift): drop `payload["deletion_conflicted"]` → "the option is passed" fails (`…-T44.log`).

- [ ] **Step 9: Gate and commit**

Task gate; expected lib count **`L0 + 61`**; Swift 96.

```bash
cd $WT && git commit -m "feat(desktop): provisional ids resolve to the server file, fetches of queued writes are served locally, unknown ids are refused" -- src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket.rs src-tauri/src/runner.rs BeebeebFileProvider/IPCFraming.swift BeebeebFileProvider/XPCBridge.swift BeebeebFileProvider/FileProviderExtension.swift BeebeebFileProviderTests/main.swift scripts/test-ipc-framing.sh
```

**Stop point:** after the commit.

---

## Task 9: Rule 5: the item's status follows the queue (spec §9.1–§9.2, m-8; commit 9)

An item with a Finder upload that has not parked is presented `uploading`: writable and not evictable, through retries and relaunches. A parked one is read-only and still not evictable. An item in the trash never looks writable.

**Files:**
- Modify: `src-tauri/src/ipc_socket.rs`: `file_entry_payload_for_db` presents the queue's status; `CAP_WRITE` (`IPC:225`) becomes `pub(crate)` for the tests
- Modify: `src-tauri/src/state_db.rs`: `reconcile_stale_in_flight_on_startup` (`SD:2041-2053`); every park (Task 2's `park_claimed`, Task 3's claim and accept parks, Task 6's `note_snapshot_for_base_pending`) settles the row's status in its own transaction
- Modify: `src-tauri/src/engine_bridge.rs`: the `Rollback` guard in `upload_version` (`EB:664-695`); the mock gains `fail_complete_once` and `trashed_files`
- Test: `engine_bridge.rs` tests, `state_db.rs` tests

**Interfaces:**
- Consumes: `ItemPresentation.unparked_finder_upload` (Task 1); the parks (Tasks 2, 3, 5, 6); `fp_save` (Task 3); `land_one_save` (Task 4).
- Produces:
  - `fn settle_status_after_park_conn<C: Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()>`: `UPDATE files SET status = 'error' WHERE file_id = ?1 AND status = 'uploading' AND NOT EXISTS (an unparked Finder upload of the file)`, recording a change when it flips
  - mock: `VersionedServerState.fail_complete_once: HashSet<String>` (by session: the next `complete` answers 500), `trashed_files: HashSet<String>` (every `init` for such a file answers `409 {"error": "file is in trash"}`, class `other`)

- [ ] **Step 1: Write the failing tests**

`state_db.rs` tests:

```rust
    #[test]
    fn startup_keeps_a_row_with_a_live_upload_uploading() {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "with-save", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("with-save").unwrap().unwrap();
        contract.current_version = 1;
        db.set_file_contract_state(&contract).unwrap();
        let accepted = db
            .accept_finder_write(
                &FinderAccept {
                    op_id: "op-with-save",
                    file_id: "with-save",
                    kind: FinderAcceptKind::Modify { incoming_base: Some("1") },
                    parent_id: None,
                    target_path: Some("with-save.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/with-save",
                    size_bytes: 3,
                    modified_at: 100,
                    backup_source_key: None,
                    now: 100,
                },
                &|_, _| Vec::new(),
            )
            .unwrap();
        assert!(matches!(accepted, AcceptOutcome::Queued { .. }));
        seed_own_row(&db, "stale-upload", FileStatus::Uploading, 20);

        db.reconcile_stale_in_flight_on_startup().unwrap();

        assert_eq!(db.get_file("with-save").unwrap().unwrap().status, FileStatus::Uploading, "its save still uploads");
        assert_eq!(db.get_file("stale-upload").unwrap().unwrap().status, FileStatus::Error, "unchanged for a row with no Finder upload");
    }
```

`engine_bridge.rs` tests (setup as in Task 3):

```rust
    #[tokio::test]
    async fn a_retrying_upload_keeps_the_item_writable() {
        // master_key [95u8; 32]
        seed_uploaded_row(&bridge, &server, "retrying");
        fp_save(&bridge, dir.path(), "retrying", "notes.txt", b"edit", "1");
        server.state.lock().unwrap().fail_first_chunk_once.insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // one transient failure
        assert_eq!(bridge.db.get_file("retrying").unwrap().unwrap().status, FileStatus::Uploading, "the row is not rolled back to Error");
        let entry = bridge.db.get_file("retrying").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_eq!(payload.status, "uploading");
        assert_ne!(payload.capabilities & crate::ipc_socket::CAP_WRITE, 0, "writable");
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_trashing_row_is_never_presented_uploading() {
        // master_key [96u8; 32]; m-8
        seed_uploaded_row(&bridge, &server, "trashed-here");
        let w = fp_save(&bridge, dir.path(), "trashed-here", "notes.txt", b"edit", "1");
        bridge.queue_finder_delete("trashed-here", w.token).unwrap();
        let entry = bridge.db.get_file("trashed-here").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_ne!(payload.status, "uploading");
        assert_eq!(payload.capabilities & crate::ipc_socket::CAP_WRITE, 0, "no write capability in the trash");
        assert_eq!(payload.parent_identifier, crate::ipc_socket::FP_TRASH_APPLE, "under the trash container");
        drop(server.finish());
    }

    #[tokio::test]
    async fn a_restart_mid_upload_resumes_and_keeps_the_token() {
        // master_key [97u8; 32]; Review Focus 2
        let db_path = dir.path().join("state.db");
        seed_uploaded_row(&bridge, &server, "restart");
        let token = fp_save(&bridge, dir.path(), "restart", "notes.txt", b"saved before the restart", "1").token.unwrap();
        server.state.lock().unwrap().fail_complete_once.insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // every chunk acknowledged, no completion
        let op_id = bridge.db.list_operations_for_file("restart").unwrap().remove(0).op_id;
        let _crashed_runner = bridge.db.claim_operation(&op_id, now_secs()).unwrap(); // the process dies holding a claim
        drop(bridge);

        // Relaunch: a new process opens the same state.db (RUN:988-997).
        let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), master_key);
        assert_eq!(bridge.db.engine_start_repair().unwrap().claims_cleared, 1);
        bridge.db.reconcile_stale_in_flight_on_startup().unwrap();
        assert_eq!(bridge.db.get_file("restart").unwrap().unwrap().status, FileStatus::Uploading, "still writable");
        assert_eq!(held_content_version(&bridge, "restart"), token, "same name: no re-download");
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.inits.len(), 1, "the session resumed: {:?}", state.init_summary());
        assert_eq!(state.latest_plaintext("restart", master_key), b"saved before the restart");
        assert_eq!(held_content_version(&bridge, "restart"), token);
    }

    #[tokio::test]
    async fn a_save_to_a_file_trashed_elsewhere_keeps_its_bytes() {
        // master_key [98u8; 32]; Review Focus 5
        seed_uploaded_row(&bridge, &server, "trashed-on-web");
        fp_save(&bridge, dir.path(), "trashed-on-web", "notes.txt", b"an edit the person made", "1");
        let trash = crate::api_client::SyncOp { seq_id: 7, op_type: "file_trash".into(), payload: serde_json::json!({ "id": "trashed-on-web" }) };
        apply_sync_op(&bridge, &sync_root, &trash, now_secs(), &mut Vec::new()).unwrap();
        server.state.lock().unwrap().trashed_files.insert("trashed-on-web".into());
        drain_upload_queue(&bridge, &sync_root).await;
        let op = bridge.db.list_operations_for_file("trashed-on-web").unwrap().remove(0);
        assert!(op.attempts < op.max_attempts, "class other: retried, not parked at once");
        assert_eq!(std::fs::read(op.payload_path.as_deref().unwrap()).unwrap(), b"an edit the person made", "the bytes are kept");
        let entry = bridge.db.get_file("trashed-on-web").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_ne!(payload.status, "uploading", "an item in the trash never looks writable");
        drop(server.finish());
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- startup_keeps_a_row_with a_retrying_upload_keeps a_trashing_row_is_never a_restart_mid_upload a_save_to_a_file_trashed_elsewhere > $EVID/r4-t9-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t9-red.log | head -20
```

Expected REDs: T31 `Error` (`SD:2041-2053`); T33 the row rolled back to `Error` (`EB:681-694`); RF2 `Error` after the reconcile; T60 and RF5 are guards against the override reaching `Trashing` rows, and go red by their mutation.

- [ ] **Step 3: Implement**

3.1 `file_entry_payload_for_db`: after the one read and `builder_seam()`, present the queue's status:

```rust
    // §9.1: a Finder upload that has not parked keeps the item writable and not evictable
    // (ITEM.h:531-533). A Trashing row keeps its trash presentation (m-8).
    let shown;
    let entry = match &presentation {
        Some(p) if p.unparked_finder_upload && entry.status != crate::state_db::FileStatus::Trashing => {
            shown = crate::state_db::FileEntry { status: crate::state_db::FileStatus::Uploading, ..entry.clone() };
            &shown
        }
        _ => entry,
    };
```

and build the payload from this `entry`. Make `CAP_WRITE` `pub(crate)`.

3.2 `reconcile_stale_in_flight_on_startup` (`SD:2041-2053`):

```sql
UPDATE files
SET status = CASE status
    WHEN 'downloading' THEN 'cloud_only'
    WHEN 'uploading' THEN CASE WHEN EXISTS (
            SELECT 1 FROM operation_queue q
            WHERE q.file_id = files.file_id AND q.write_id IS NOT NULL
              AND q.kind IN ('upload_version', 'upload_file') AND q.attempts < q.max_attempts)
        THEN 'uploading' ELSE 'error' END
    ELSE status
END
WHERE status IN ('downloading', 'uploading')
```

Update its doc comment (`SD:2033-2040`): a row with a Finder upload that has not parked stays `Uploading`.

3.3 `settle_status_after_park_conn` and its callers. Each park runs in a transaction and calls it for the parked op's file after the op's `UPDATE`: `park_claimed` (wrap its statement in a transaction, read the op's `file_id`), the claim's park (Task 3 `park_in_claim`), the accept's park (rule 6a′, after the `INSERT`), and `note_snapshot_for_base_pending` (for each parked file).

3.4 The `Rollback` guard (`EB:664-695`): add `finder_write: bool` (`claim.and_then(|c| c.write.as_ref()).is_some()`); its `drop` returns at once when `finder_write` is true. The row stays `Uploading` while the op will retry; a park sets `Error` (3.3). Keep Mine (`claim: None`) and Windows uploads keep today's rollback.

3.5 Mock: `fail_complete_once` applies to `POST /api/v1/uploads/{session}/complete` (remove the session from the set and answer `500 {"error": "boom"}`, keeping the session); `trashed_files` applies to `POST /api/v1/uploads/init` whose `file_id` is in the set (`409 {"error": "file is in trash"}`, recorded in `inits`).

- [ ] **Step 4: GREEN, mutations**

Step-2 command into `$EVID/r4-t9-green.log`; expected `ok. 5 passed`. Mutations (`$EVID/r4-t9-mut-<T>.log`):
- T31: today's reconcile SQL → `Error`.
- T33: keep the rollback to `Error` for Finder writes → "the row is not rolled back to Error" fails.
- T60: apply the override to `Trashing` rows → `uploading`.
- RF2: today's reconcile SQL → "still writable" fails.
- RF5: classify `"file is in trash"` as `StaleBase` → parked at once.

- [ ] **Step 5: Gate and commit**

Task gate; expected lib count **`L0 + 66`**. The existing reconcile test (`SD:5352-5372`) stays green: its rows have no Finder upload.

```bash
cd $WT && git commit -m "feat(desktop): an item with a queued Finder save stays writable and is never evicted" -- src-tauri/src/ipc_socket.rs src-tauri/src/state_db.rs src-tauri/src/engine_bridge.rs
```

**Stop point:** after the commit.

---

## Task 10: Rule 6: the upgrade path (spec §10.1–§10.3, m-4c; commit 10)

Ops an earlier build queued get a write id and are marked earlier-build, so they are never handed over and never treated as minted. Their bases are left alone: a timestamp base meets 409 and parks with its bytes (ruling 2).

**Files:**
- Modify: `src-tauri/src/state_db.rs`: new `mark_earlier_build_uploads`
- Modify: `src-tauri/src/runner.rs`: call it at engine start, after `engine_start_repair` and before `reconcile_stale_in_flight_on_startup` (`RUN:997`), on macOS only (plan Spec issue 13)
- Test: `engine_bridge.rs` tests, `state_db.rs` tests

**Interfaces:**
- Consumes: `WriteOrigin`, `mint_write_id` (Task 1); the stale-base park (Task 5).
- Produces: `StateDb::mark_earlier_build_uploads(&self) -> Result<usize>` (the number of ops marked).

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn an_old_timestamp_base_parks_at_its_first_attempt() {
        // engine_bridge tests; master_key [99u8; 32]; the two QA shapes (spec §2 item 11)
        seed_uploaded_row(&bridge, &server, "qa-89da");
        // The way the round-3 QA build queued it: through the shared entry point, no write id,
        // a base that is a wall-clock second (QA/inv2-statedb-copy-queries.log).
        let contents = dir.path().join("old.txt");
        std::fs::write(&contents, b"forty bytes the old build queued, here.").unwrap();
        bridge.queue_finder_modify(finder_file_target(Some("qa-89da"), "notes.txt", &contents, Some("1791550250".into()))).unwrap();
        assert_eq!(bridge.db.mark_earlier_build_uploads().unwrap(), 1);
        let op = bridge.db.list_operations_for_file("qa-89da").unwrap().remove(0);
        assert_eq!(op.base_version, Some(1_791_550_250), "no re-base");
        assert_eq!(bridge.db.finder_write(&op.op_id).unwrap().unwrap().origin, crate::state_db::WriteOrigin::EarlierBuild);
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.init_summary(), vec![(json!("qa-89da"), json!(1_791_550_250_i64), 409)], "one init with the timestamp base");
        let op = bridge.db.get_operation(&op.op_id).unwrap().unwrap();
        assert_eq!(op.attempts, op.max_attempts, "parked at its first attempt");
        assert!(std::path::Path::new(op.payload_path.as_deref().unwrap()).is_file(), "its bytes are kept");
        assert!(bridge.db.item_presentation("qa-89da").unwrap().unwrap().held.is_none(), "no held token");
    }
```

```rust
    #[test]
    fn a_re_upgrade_clears_stale_held_columns_and_waiting_ops_store_b() {
        // state_db tests; m-4c (and m-4a, already true since Task 3)
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed_own_row(&db, "downgraded", FileStatus::Local, 10);
        let mut contract = db.get_file_contract_state("downgraded").unwrap().unwrap();
        contract.current_version = 2;
        db.set_file_contract_state(&contract).unwrap();
        let accept = |op_id: &'static str, base: &'static str| {
            db.accept_finder_write(
                &FinderAccept {
                    op_id, file_id: "downgraded", kind: FinderAcceptKind::Modify { incoming_base: Some(base) },
                    parent_id: None, target_path: Some("d.txt"), metadata_json: "{}", payload_path: "/staged/d",
                    size_bytes: 1, modified_at: 1, backup_source_key: None, now: 1,
                },
                &|_, _| Vec::new(),
            )
            .unwrap()
        };
        let AcceptOutcome::Queued { token, .. } = accept("w1", "2") else { panic!("queued") };
        let AcceptOutcome::Queued { .. } = accept("w2", Box::leak(token.into_boxed_str())) else { panic!("queued") };
        assert_eq!(db.get_operation("w2").unwrap().unwrap().base_version, Some(2), "m-4a: the waiting op stores its b");
        // A downgraded build then queued an upload of the same file without a write id.
        db.enqueue_operation(&queued("older-build", OperationKind::UploadVersion, "downgraded", None)).unwrap();
        db.mark_earlier_build_uploads().unwrap();
        assert!(db.item_presentation("downgraded").unwrap().unwrap().held.is_none(), "m-4c: the held columns are cleared");
        assert_eq!(db.finder_write("older-build").unwrap().unwrap().origin, WriteOrigin::EarlierBuild);
    }
```

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- an_old_timestamp_base_parks a_re_upgrade_clears_stale_held > $EVID/r4-t10-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t10-red.log | head
```

Expected: `mark_earlier_build_uploads` not found; with a stub returning `0`, T34 fails at `assert_eq!(…, 1)` and T66 at "m-4c".

- [ ] **Step 3: Implement (`state_db.rs`)**

```rust
    /// §10.2–§10.3: every upload an earlier build queued gets a write id and is marked
    /// earlier-build; its base is left as it is. First (m-4c) the held columns of its file
    /// are cleared: a re-upgrade must not revive a token the system no longer holds.
    pub fn mark_earlier_build_uploads(&self) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL, held_object_version_id = NULL
             WHERE file_id IN (SELECT file_id FROM operation_queue
                               WHERE write_id IS NULL AND file_id IS NOT NULL
                                 AND kind IN ('upload_version', 'upload_file'))",
            [],
        )?;
        let ops: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT op_id FROM operation_queue
                 WHERE write_id IS NULL AND kind IN ('upload_version', 'upload_file') ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        for op_id in &ops {
            tx.execute(
                "UPDATE operation_queue SET write_id = ?2, write_origin = 'earlier_build' WHERE op_id = ?1",
                params![op_id, crate::write_token::mint_write_id()],
            )?;
        }
        tx.commit()?;
        Ok(ops.len())
    }
```

`runner.rs`, after the `engine_start_repair` block (Task 2 step 6):

```rust
    #[cfg(target_os = "macos")]
    match db.mark_earlier_build_uploads() {
        Ok(0) => {}
        Ok(marked) => tracing::info!(marked, "engine start: uploads from an earlier build marked; their bases are unchanged"),
        Err(e) => tracing::warn!(error = %e, "engine start: marking earlier-build uploads failed"),
    }
```

- [ ] **Step 4: GREEN, mutations**

Step-2 command into `$EVID/r4-t10-green.log`; expected `ok. 2 passed`. Mutations (`$EVID/r4-t10-mut-<T>.log`):
- T34: re-base a timestamp base (`>= 1_000_000_000`) to the row's `current_version` while marking (the struck §10.3 rule) → one init with base 1, `201`.
- T66: drop the held-columns `UPDATE` → "m-4c" fails.

- [ ] **Step 5: Gate and commit**

Task gate; expected lib count **`L0 + 68`**.

```bash
cd $WT && git commit -m "feat(desktop): mark uploads from an earlier build at engine start and leave their bases alone" -- src-tauri/src/state_db.rs src-tauri/src/runner.rs src-tauri/src/engine_bridge.rs
```

**Stop point:** after the commit.

---

## Task 11: Rule 7: every new log line is captured and clean, M5, the extension's options line (spec §11, §8.8 m-16; commit 11)

Each line of §11 is already emitted by the task that added its event. This task proves each one: the message, its fields, and that a planted file name and path never appear.

**Files:**
- Modify: `src-tauri/src/engine_bridge.rs` and `src-tauri/src/ipc_socket.rs`: `loggable_id` (M5) used by every line that logs an id taken from the wire, including the existing hydrate warning (`IPC:2194-2203`)
- Modify: `BeebeebFileProvider/FileProviderExtension.swift`: the options line in `createItem` and `modifyItem`
- Test: `engine_bridge.rs` tests

**Interfaces:**
- Consumes: every log helper of Tasks 2–8.
- Produces: `pub(crate) fn loggable_id(id: &str) -> &str` (the id when it parses as a UUID, else `"non_uuid"`), and the rule that a wire id is logged as `id = …` when it is a UUID and as `id_kind = "non_uuid"` otherwise.

- [ ] **Step 1: Write the failing tests**

A helper and the scenario name all twelve tests share:

```rust
    /// The planted name and the staging path must never reach a log line.
    const PLANTED: &str = "Quarterly secret plan.txt";

    fn assert_clean_line(logs: &str, message: &str, fields: &[&str], dir: &Path) -> String {
        let line = logs
            .lines()
            .find(|line| line.contains(message))
            .unwrap_or_else(|| panic!("no `{message}` line:\n{logs}"))
            .to_string();
        for field in fields {
            assert!(line.contains(&format!("{field}=")), "`{message}` lacks `{field}`: {line}");
        }
        for leak in ["Quarterly", "secret", "/api/v1", "127.0.0.1", "/Users/", &dir.to_string_lossy()] {
            assert!(!line.contains(leak), "`{message}` leaks `{leak}`: {line}");
        }
        line
    }
```

T37, one test per new line. Each reuses the scenario of the test named in the middle column, with `PLANTED` as the file name wherever a name is passed, and wraps the step that emits the line in `capture_logs_async`:

| Test | Scenario (from) | Message | Fields |
|---|---|---|---|
| `log_finder_write_refused_unknown_item` | T17 | `Finder write refused` | `op`, `reason` (= `unknown_item`) |
| `log_upload_refused_with_its_class` | T25 | `upload refused by the server (409 Conflict); will retry` | `op_id`, `file_id`, `attempt`, `max_attempts`, `class` (= `in_progress`) |
| `log_parked_with_its_reason` | T23 (the predecessor's park) | `upload parked with its bytes kept in the queue` | `op_id`, `file_id`, `reason` (= `payload_missing`) |
| `log_took_over` | T23 | `queued write took over a parked one` | `op_id`, `file_id`, `parked_op_id` |
| `log_alias_resolved` | T15 | `provisional id resolved to the server id` | `request` (= `modify`), `provisional_id`, `file_id` |
| `log_alias_delete_kept` | T58 | `delete of a provisional id not applied to the server file` | `provisional_id`, `file_id` |
| `log_base_from_snapshot` | T11 (with `apply_snapshot` called directly) | `queued write based on the version the snapshot reported` | `op_id`, `file_id`, `base_version` |
| `log_version_raised` | T48 | `file version raised from the snapshot` | `file_id`, `old_version`, `new_version` |
| `log_queue_state_moved` | T42 | `queue state moved` | `op_id`, `step` |
| `log_completed_landing_retried` | P9 (one injected failure) | `upload completed on the server; local landing will be retried` | `op_id`, `file_id`, `attempt` |
| `log_hydrate_served` | T18 (`source=queue`) and RF3 | `Finder hydrate served` (`INFO`) | `file_id`, `source` |

Example, the first row in full:

```rust
    #[tokio::test]
    async fn log_finder_write_refused_unknown_item() {
        // master_key [100u8; 32]
        let contents = dir.path().join(PLANTED);
        std::fs::write(&contents, b"edit").unwrap();
        let logs = capture_logs_async(async {
            let result = bridge.queue_file_provider_modify(finder_file_target(Some("3f2a9c1e-0000-4000-8000-00000000beef"), PLANTED, &contents, Some("1".into())));
            let _ = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
        })
        .await;
        let line = assert_clean_line(&logs, "Finder write refused", &["op", "reason"], dir.path());
        assert!(line.contains("unknown_item"), "{line}");
        drop(server.finish());
    }
```

T38:

```rust
    #[tokio::test]
    async fn m5_a_non_uuid_wire_id_is_never_logged() {
        // master_key [101u8; 32]
        assert_eq!(loggable_id("3f2a9c1e-0000-4000-8000-000000000001"), "3f2a9c1e-0000-4000-8000-000000000001");
        assert_eq!(loggable_id("../../etc/passwd"), "non_uuid");
        let contents = dir.path().join("x.txt");
        std::fs::write(&contents, b"x").unwrap();
        let logs = capture_logs_async(async {
            // An unknown non-UUID id on the write path …
            let result = bridge.queue_file_provider_modify(finder_file_target(Some("not-a-uuid/../evil"), "x.txt", &contents, Some("1".into())));
            let _ = crate::ipc_socket::write_outcome_response("modify", &bridge.db, result);
            // … and the hydrate failure line, which on Linux logs a caller-supplied id (IPC:2064-2067, IPC:2195-2199).
            let _ = crate::ipc_socket::hydrate_failure_reply("not-a-uuid/../evil", anyhow::anyhow!("boom"));
        })
        .await;
        assert!(!logs.contains("not-a-uuid"), "{logs}");
        assert!(logs.contains("non_uuid"), "{logs}");
        drop(server.finish());
    }
```

(`hydrate_failure_reply` becomes `pub(crate)`. Write the other ten T37 tests in the same shape as the first; each is a copy of its scenario test's setup with the capture around the emitting step.)

- [ ] **Step 2: Run them; expect RED**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- log_ m5_a_non_uuid > $EVID/r4-t11-red.log 2>&1; echo "rc=$?"; grep -E "error\[|FAILED|panicked|test result" $EVID/r4-t11-red.log | head -20
```

Expected: `loggable_id` not found; T38 then fails with the raw id in the refused line and the hydrate line. Any T37 row that fails on a missing field or a leak is a real defect in its owning task's line: fix the line here and say so in Notes. A T37 row that passes at once is expected (its line exists since its task); its RED comes from its mutation.

- [ ] **Step 3: Implement**

3.1 M5 (`engine_bridge.rs`):

```rust
/// Spec §11 M5: an id taken from the wire is logged only when it parses as a UUID.
pub(crate) fn loggable_id(id: &str) -> &str {
    if uuid::Uuid::parse_str(id).is_ok() { id } else { "non_uuid" }
}
```

Every line that logs an id from the wire passes it through `loggable_id`, and logs it as `id_kind = "non_uuid"` instead of the id field when `loggable_id` returns `"non_uuid"`: the refused write (log the requested id), `log_alias_resolved` and `log_alias_delete_kept` (`provisional_id`), `log_hydrate_served` (`file_id`), and `hydrate_failure_reply` (`IPC:2194-2203`).

3.2 The options line (`FileProviderExtension.swift`), first statement of `createItem` and of `modifyItem`:

```swift
        if options.rawValue != 0 {
            // Spec §8.8 (m-16): recorded for D9; the write is handled as usual.
            NSLog("BeebeebFileProvider: write options call=%@ options=%lu item=%@",
                  "create", UInt(options.rawValue), itemTemplate.itemIdentifier.rawValue)
        }
```

(`"modify"` and `item.itemIdentifier.rawValue` in `modifyItem`.) It is checked on the device in D9 (plan Spec issue 10); Swift stays at 96 tests.

- [ ] **Step 4: GREEN, mutations**

```bash
cd $WT/src-tauri && $LOCK cargo-build -- cargo test --locked -p beebeeb-desktop --lib -- log_ m5_a_non_uuid > $EVID/r4-t11-green.log 2>&1; echo "rc=$?"; grep "test result:" $EVID/r4-t11-green.log
cd $WT && bash scripts/test-ipc-framing.sh > $EVID/r4-t11-swift.log 2>&1; tail -1 $EVID/r4-t11-swift.log
```

Expected `ok. 12 passed`, Swift `96 passed, 0 failed`. Mutations: for each T37 row, add the planted name to its line (for example `name = %display_name`) and see that row fail on "leaks `Quarterly`" (`$EVID/r4-t11-mut-<test>.log`, eleven logs); T38: log the raw id → "not-a-uuid" appears.

- [ ] **Step 5: Gate and commit**

Task gate; expected lib count **`L0 + 80`**. Also the extension build (Task 8 step 8's command) → `rc=0`.

```bash
cd $WT && git commit -m "test(desktop): every new log line carries ids and categories only; wire ids that are not UUIDs are never logged" -- src-tauri/src/engine_bridge.rs src-tauri/src/ipc_socket.rs BeebeebFileProvider/FileProviderExtension.swift
```

**Stop point:** after the commit.

---

## Task 12: Docs that change with the code (spec §17; commit 12)

**Files:**
- Modify: `docs/IPC_PROTOCOL.md`: "Versions and queued writes" (`IPC_PROTOCOL.md:143-266`), "Write-queue idempotency" (`:282-408`, the `QueueFinderCreate` fields), "Thumbnails" (`:568-596`), "Version skew" (`:597-612`), "Tests" (`:613-`)
- Check: `CLAUDE.md` ("Current macOS integration state"): `grep -n "content version\|contentVersion\|version identifier" CLAUDE.md` printed nothing at `e7e3dff`. If it still prints nothing, no change; write the empty grep result into Notes.

**Interfaces:** none.

- [ ] **Step 1: Rewrite "Versions and queued writes"**, in this order, each with the spec section it states:
  1. The write token: format, minting, matching (§5.1); what is stored (§5.2, without the split pieces).
  2. The predicate and its one read; who reports it; the guarded clear (§5.3); the state table (§5.4) as one Mermaid `stateDiagram-v2` (the spec's, without the split pieces).
  3. The base mapping, rules 1a–6b, with rule 1c marked "split off to task 1879; a lost reply parks" (§6.1); the `init` guard and `base_pending` (§6.3).
  4. The chain by write id, the claim, the hand-over at the claim and its one-pass cost, the 409 classes and the immediate parks (§8.1–§8.5).
  5. The landing transaction and the completion record (§8.6 rules 1–4, 6; rule 5 named as split off).
  6. Serialization (§8.7 S1–S6) as a short list, with the Mermaid sequence of S5.
  7. Provisional ids: the alias, what resolves through it, the gated delete, the deletion-conflicted create, unknown ids (§7.1–§7.3); fetches served from the queue (§7.4).
  8. Status follows the queue (§9).
  9. The log table (§11, without `pinned` and `create_replay`).
  Replace the three passages the spec says it replaces (its "Replaces" line): the "rebase on an equal base" chain, the "queued, no item" reply row, and the "Not covered" note on provisional ids.
- [ ] **Step 2: `QueueFinderCreate`**: document `deletion_conflicted` (bool, default `false`), `template_identifier` and `template_content_version` (both optional, sent only when `deletion_conflicted` is true); an older daemon ignores them and keeps today's create.
- [ ] **Step 3: "Thumbnails"**: while a held write is queued, `FetchThumbnail` answers `Error` with no server request (the safe default; the render from staged bytes is task 1879).
- [ ] **Step 4: "Version skew"**: an older extension is unaffected (the token is opaque to it: `FPI:222-226`, `FPE:592-594`); an older daemon reads the token's first segment as the base; `0:wW` after a create landed is the one downgrade hole, and a queued save can be downloaded over by an older build until it lands (§5.6 m-4b; release notes).
- [ ] **Step 5: "Tests"**: one row per T-number of this plan with its test name and file, and T28, T32, T51, T56, T59, T63 listed as "moved to task 1879".
- [ ] **Step 6: Check and commit**

```bash
cd $WT && grep -c "pinned=\|create_replay\|minted_writes\|held_mtime" docs/IPC_PROTOCOL.md   # expected 0, except in the "split off" lines
grep -n '```mermaid' docs/IPC_PROTOCOL.md | wc -l                                               # at least 2 (state, S5)
git diff --stat 9eced4a -- src-tauri/Cargo.lock bun.lock                                        # empty: no new dependency
git commit -m "docs(ipc): versions and queued writes after round 4: the write token, the claim, the hand-over, aliases" -- docs/IPC_PROTOCOL.md
```

**Stop point:** after the commit. The lane's work ends here; Task 13 is the lead's.

---

## Task 13: The device plan, D1–D10 (spec §14 as amended in 4c; lead only)

**Lead only, on the lead's Mac, by the lead.** No lane runs, quits or drives `/Applications/Beebeeb.app`, Finder, System Settings, CloudStorage or the containers. Everything runs against the local API with a test account. Nothing touches production. Evidence goes to `$EVID` as `R5-*`.

D1's log check follows plan Spec issue 14.

- [ ] **Step 1: Gates at the head, in a fresh tree carrying only this branch**

```bash
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop fetch origin
git -C /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/desktop worktree add --detach ~/code/bb-worktrees/desktop-1873-r4-gate <head of the lane's branch>
G=~/code/bb-worktrees/desktop-1873-r4-gate
cd $G/src-tauri && $LOCK cargo-build -- cargo test --locked > $EVID/r4-final-cargo-test.log 2>&1; echo $? > $EVID/r4-final-cargo-test.exit
python3 ../scripts/assert-cargo-test-counts.py $EVID/r4-final-cargo-test.log --cargo-exit-code "$(cat $EVID/r4-final-cargo-test.exit)"
grep "test result:" $EVID/r4-final-cargo-test.log                        # lib: ok. L0+80 passed; 0 failed; 4 ignored; keychain 20; wsw 8; wsc 3
$LOCK cargo-build -- cargo clippy --locked --all-targets > $EVID/r4-final-clippy.log 2>&1
grep -c '^warning' $EVID/r4-final-clippy.log                              # ≤ the baseline (146, Task 1 step 0)
cd $G && bash scripts/test-ipc-framing.sh > $EVID/r4-final-swift.log 2>&1; tail -1 $EVID/r4-final-swift.log   # 96 passed, 0 failed (expected 96)
bash scripts/test-ipc-framing.sh --self-test > $EVID/r4-final-swift-selftest.log 2>&1; echo "rc=$?"
python3 scripts/check-ipc-timeouts.py && python3 scripts/check-ipc-timeouts.py --self-test; echo "rc=$?"
git diff --stat 9eced4a -- src-tauri/Cargo.lock bun.lock                   # empty
```

- [ ] **Step 2: Build the signed QA app from the gate tree** (the recipe that built the 1834 and 1873 QA apps; the updater step is switched off so the build ends at `rc=0`)

```bash
security find-identity -v -p codesigning | grep "Apple Development"
export APPLE_SIGNING_IDENTITY="<the Apple Development identity printed above>"
export MACOS_APP_PROVISION_PROFILE=~/Downloads/beebeeb-qa-app.provisionprofile          # the QA profiles used for 1834/1873
export MACOS_FILE_PROVIDER_PROVISION_PROFILE=~/Downloads/beebeeb-qa-fileprovider.provisionprofile
cd $G && bun install --frozen-lockfile
$LOCK cargo-build -- bunx tauri build --debug --target aarch64-apple-darwin --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' -- --locked > $EVID/R5-qa-build.log 2>&1; echo "rc=$?"
APPQA=$G/src-tauri/target/aarch64-apple-darwin/debug/bundle/macos/Beebeeb.app
codesign --verify --deep --strict "$APPQA" && echo "verify OK"
codesign -d --entitlements - "$APPQA" 2>&1 | grep -c app-sandbox          # 1
ls "$APPQA/Contents/PlugIns/"                                             # BeebeebFileProvider.appex
```

(If the profile file names differ, list `~/Downloads/beebeeb-qa-*.provisionprofile` and use the two the 1873 round-3 build embedded.)

- [ ] **Step 3: Environment, captures and helpers**

Precondition: the installed app is the `931b885` QA build (round 3's rung), its Finder location is kept, and the two old ops are still in its queue (D1). Do not reset anything before D1.

```bash
cd /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/server && cargo run -p beebeeb-api --bin beebeeb-api   # its own terminal; `make dev-native` cannot start the API (progress ledger, 2026-10-09 09:13)
curl -sf localhost:3001/health
launchctl setenv BB_API_BASE http://localhost:3001                         # unset in step 16
EVID=/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/.claude/tasks/_qa-evidence/1873
ROOT="$HOME/Library/CloudStorage/Beebeeb-Beebeeb"
PSQL="psql postgresql://beebeeb:beebeeb_dev@localhost:5434/beebeeb -At"
QA_TMP=$(mktemp -d)                                                       # scratch for big files, the D8b helper and the CLI's isolated HOME
dd if=/dev/urandom of=$QA_TMP/big-1200m.bin bs=1m count=1200 2>/dev/null
dd if=/dev/urandom of=$QA_TMP/big-600m.bin bs=1m count=600 2>/dev/null
dd if=/dev/urandom of=$QA_TMP/big-250m.bin bs=1m count=250 2>/dev/null

start_step() {   # $1 = step; instrument 1, captured live (spec §14)
  date '+%Y-%m-%d %H:%M:%S' > $EVID/R5-$1.start
  /usr/bin/log stream --style compact --predicate 'process == "fileproviderd" AND eventMessage CONTAINS "fetch-content("' > $EVID/R5-$1-fetch.log 2>&1 &
  echo $! > $EVID/R5-$1-fetch.pid
}
end_step() {     # $1 = step
  sleep 3; kill "$(cat $EVID/R5-$1-fetch.pid)"
  /usr/bin/log show --start "$(cat $EVID/R5-$1.start)" --predicate 'process == "fileproviderd" OR process == "BeebeebFileProvider"' > $EVID/R5-$1-fp.log 2>&1
  $PSQL -c "SELECT id, size_bytes, version_number, is_uploading, updated_at FROM files ORDER BY updated_at DESC LIMIT 30" > $EVID/R5-$1-devdb.txt
  sqlite3 "$STATE_DB" ".backup '$EVID/R5-$1-state.db'"
}
id_of() {        # $1 = path under the root, $2 = a state.db copy: the item id the app knows it by
  sqlite3 "$2" "SELECT file_id FROM files WHERE path = '$1'"
}
fetches() { grep -c "done executing.*fetch-content($1)" $EVID/R5-$2-fetch.log; }       # instrument 1
served()  { grep -c "Finder hydrate served.*file_id=$1" $EVID/R5-app-stdout.log; }      # instrument 2
sample()  {      # $1 = file, $2 = step: ls -l, size and the last 16 bytes, every second
  ( while :; do date +%T; ls -lO "$1"; wc -c < "$1"; tail -c 16 "$1" | xxd -p; sleep 1; done ) > $EVID/R5-$2-samples.txt 2>&1 &
  echo $! > $EVID/R5-$2-samples.pid
}
stop_sample() { kill "$(cat $EVID/R5-$1-samples.pid)"; }
```

```bash
# The state dir the app logs at start (QA/R0-app-stdout.log: "state dir resolved … state_dir=…"); same container for the QA build.
STATE_DB="$HOME/Library/Containers/io.beebeeb.app/Data/Library/Application Support/io.beebeeb.app/state.db"
ls -l "$STATE_DB"     # confirm; after step 4, also confirm the new build logged the same state_dir
```

`kill` only ever targets PIDs this procedure recorded.

**The CLI session for D4b and D8b**, signed in once here with an isolated `HOME`, so the CLI never reads or writes the operator's own config (it holds a live session and master key):

```bash
BB="$(command -v bb || echo cargo run --quiet --manifest-path /Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/cli/Cargo.toml --)"
HOME=$QA_TMP $BB --api http://localhost:3001 login        # approve in the local web app, signed in as the test account
CFG="$QA_TMP/Library/Application Support/beebeeb/config.json"   # dirs::config_dir under the isolated HOME (repos/cli/src/config.rs:30-33)
TOKEN=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["session_token"])' "$CFG")
MK=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["master_key"])' "$CFG")
```

**The positive control** (spec §14, "Fetch counting"). Before every step whose pass contains "no fetch" (D2, D3, D4, D4b, D5, D6):

```bash
control() {      # $1 = step. ctl-<step>.txt was created in step 4 and has landed (materialized, untouched).
  CTL_ID=$(id_of "ctl-$1.txt" $EVID/R5-setup-state.db)
  # In Finder: right-click ctl-<step>.txt → Remove Download. Then:
  cat "$ROOT/ctl-$1.txt" > /dev/null
  sleep 3
  echo "control $1: instrument1=$(fetches $CTL_ID $1) instrument2=$(grep -c "Finder hydrate served file_id=$CTL_ID source=server" $EVID/R5-app-stdout.log)"
}
```

Pass: exactly `instrument1=1 instrument2=1`. A void control changes the instrument; it is never repeated as is: (1) the live stream above; (2) the same predicate with `/usr/bin/log stream --level debug`; (3) instrument 2 alone, and the step's result is then written "no fetch reached the daemon", with the system-side gap stated in the evidence. Queue-served fetches have no control (an item with a queued write is not evictable, §9.1): for them, instrument 1 plus the `source=queue` field (T37) is the evidence.

**Greps used in every step** (`<id>` the item, `<step>` the step):

| Check | Command | Pass |
|---|---|---|
| no re-download of our own bytes | `served <id>`; `fetches <id> <step>`; `grep -c "evictWithOldVersion.*<id>" $EVID/R5-<step>-fp.log` | 0, 0, 0 after the save's `update-item` (and the control counted 1 in both instruments) |
| one name for our bytes | `grep -o "s:<id>[^>]*cver:[^ ]*" $EVID/R5-<step>-fp.log \| sort -u` | one value from the reply on (it may print as `{blobNN}`) |
| never read-only | `grep -cE "s:<id>.*m:r--" $EVID/R5-<step>-fp.log`; the samples | 0; every `ls -lO` sample `-rw-` |
| no refusal | `grep -E "409 Conflict\|Finder hydrate failed\|Finder write refused" $EVID/R5-app-stdout.log \| grep -c <id>` | 0 |
| bytes | `shasum -a 256 "$ROOT/<file>"` | the hash of the bytes the step wrote |

- [ ] **Step 4: D1, the upgrade with the two old ops**

Before installing, with the `931b885` build still running:

```bash
$PSQL -c "SELECT id, size_bytes, version_number FROM files WHERE id LIKE '89da2556%' OR id LIKE 'bdfa53ed%'" > $EVID/R5-D1-devdb-before.txt
sqlite3 "$STATE_DB" ".backup '$EVID/R5-D1-state-before.db'"
sqlite3 $EVID/R5-D1-state-before.db "SELECT op_id, attempts, max_attempts, base_version, payload_path FROM operation_queue WHERE op_id LIKE '905b6d55%' OR op_id LIKE 'e5cec976%'" > $EVID/R5-D1-ops-before.txt
```

Expected before: `89da2556…` v1 28 B and `bdfa53ed…` v1 6 B; the two ops, with their attempts. Then install and launch:

```bash
start_step D1
osascript -e 'quit app "Beebeeb"'; sleep 5
mkdir -p ~/bb-qa/1873-r5 && ditto /Applications/Beebeeb.app ~/bb-qa/1873-r5/Beebeeb-931b885.app
ditto "$APPQA" /Applications/Beebeeb.app
RUST_LOG=info /Applications/Beebeeb.app/Contents/MacOS/beebeeb-desktop >> $EVID/R5-app-stdout.log 2>&1 &
echo $! > $EVID/R5-app.pid
sleep 75                                                                   # two passes (RUN:117)
end_step D1
grep -E "905b6d55|e5cec976" $EVID/R5-app-stdout.log > $EVID/R5-D1-lines.txt
sqlite3 $EVID/R5-D1-state.db "SELECT op_id, attempts, max_attempts, write_id IS NOT NULL, write_origin, base_version, payload_path FROM operation_queue WHERE op_id LIKE '905b6d55%' OR op_id LIKE 'e5cec976%'" > $EVID/R5-D1-ops-after.txt
```

Pass (ruling 2; Spec issue 14): for each op that had not used up its attempts before, one line in `R5-D1-lines.txt` with `409`, `class="stale_base"` and `parked`; an op already parked under the old build logs nothing. The dev DB is unchanged (v1 28 B, v1 6 B). `R5-D1-ops-after.txt` shows both ops with `attempts = max_attempts`, a write id, `write_origin = earlier_build`, and their payloads present at 40 B and 24 B (`wc -c` on each path). Both items are `r--` with the error badge (screenshot `R5-D1-items.png`). Every other upload of the session proceeds (D2 onwards).

Then the setup for the controls: create `ctl-D2.txt`, `ctl-D3.txt`, `ctl-D4.txt`, `ctl-D4b.txt`, `ctl-D5.txt`, `ctl-D6.txt` in `$ROOT` (`printf 'control %s\n' D2 > "$ROOT/ctl-D2.txt"`, …), wait 75 s, and take `sqlite3 "$STATE_DB" ".backup '$EVID/R5-setup-state.db'"`.

- [ ] **Step 5: D2, a create and two saves 1 s apart, and the landing** (R1 again)

```bash
start_step D2; control D2
printf 'D2 created\n' > "$ROOT/d2.txt"; sleep 0.5
sqlite3 "$STATE_DB" ".backup '$EVID/R5-D2-p.db'"; P=$(id_of d2.txt $EVID/R5-D2-p.db)   # the provisional id
sample "$ROOT/d2.txt" D2
printf 'save one\n' >> "$ROOT/d2.txt"; sleep 1; printf 'save two\n' >> "$ROOT/d2.txt"
printf 'D2 created\nsave one\nsave two\n' > $EVID/R5-D2-expected.txt
sleep 100; stop_sample D2; end_step D2
S=$(id_of d2.txt $EVID/R5-D2-state.db)
```

Pass: every sample 29 B and `-rw-`; the dev DB has S at v3, 29 B (three uploads: the create and one per save; the last wins); `shasum -a 256 "$ROOT/d2.txt"` equals `R5-D2-expected.txt`'s; the "one name" and "no refusal" checks pass from the first save on; instruments 1 and 2: `fetches $P D2` = 0 and `served $P` = 0 after the first save; `fetches $S D2` ≤ 1 and `served $S` ≤ 1 (the swap's materialization, recorded for D10); any other fetch fails the step. (The `stat` check and the PNG step moved to 1879.)

- [ ] **Step 6: D3, C1 forced**

```bash
start_step D3; control D3
T=$(id_of t.txt $EVID/R5-setup-state.db); V=$($PSQL -c "SELECT version_number FROM files WHERE id = '$T'")
sample "$ROOT/t.txt" D3
printf 'D3 first\n' >> "$ROOT/t.txt"; cp $QA_TMP/big-1200m.bin "$ROOT/d3-big.bin"
until [ "$($PSQL -c "SELECT version_number FROM files WHERE id = '$T'")" = "$((V+1))" ]; do sleep 1; done
$PSQL -c "SELECT is_uploading FROM files WHERE size_bytes > 1000000000 ORDER BY updated_at DESC LIMIT 1" > $EVID/R5-D3-big-uploading.txt   # t
printf 'D3 second\n' >> "$ROOT/t.txt"; cp "$ROOT/t.txt" $EVID/R5-D3-expected.txt
sleep 80; stop_sample D3; end_step D3
```

Pass: within two passes `t.txt` is at v+2; `shasum` of the local file equals `R5-D3-expected.txt`'s; no 409 line for `$T`; every sample `-rw-`; one name for its bytes after each save; no fetch of `$T` (both instruments, control passed). `R5-D3-big-uploading.txt` reads `t` (the second save landed while the big upload ran).

- [ ] **Step 7: D4, a save during the create's upload**

```bash
start_step D4; control D4
cp $QA_TMP/big-600m.bin "$ROOT/d4-big.bin"
until [ "$($PSQL -c "SELECT count(*) FROM files WHERE size_bytes = 629145600 AND is_uploading")" = "1" ]; do sleep 1; done
printf 'D4 tail\n' >> "$ROOT/d4-big.bin"; shasum -a 256 "$ROOT/d4-big.bin" | cut -d' ' -f1 > $EVID/R5-D4-expected.sha
sleep 150; end_step D4
```

Pass: exactly one new server file (`$PSQL -c "SELECT count(*) FROM files WHERE size_bytes IN (629145600, 629145608)"` = 1), at v2, 629145608 B; never `r--` or `deco:error` for its ids; no `Finder hydrate failed`; the latest version's bytes are checked after D7's read-back. Record whether the system fetched the provisional id (both instruments); if it did, the fetch succeeded and the app logged `Finder hydrate served … source=queue`.

- [ ] **Step 8: D4b, a save during the create's queue wait** (R2 again)

```bash
start_step D4b; control D4b
grep "sync operation queue processed\|completed_sync_work" $EVID/R5-app-stdout.log | tail -1      # the last pass; start right after one
printf 'D4b created\n' > "$ROOT/d4b.txt"; sleep 1; printf 'D4b appended\n' >> "$ROOT/d4b.txt"
sleep 100; end_step D4b
S=$(id_of d4b.txt $EVID/R5-D4b-state.db)
curl -sf -H "Authorization: Bearer $TOKEN" http://localhost:3001/api/v1/files/$S/versions > $EVID/R5-D4b-versions.json   # $TOKEN: step 3's CLI session
```

Pass (ruling 1, m-15): two `init`s for the file in order: one server row; version 1 = the created bytes (12 B), version 2 = the appended bytes (25 B), the last wins; no 409; the item stays `-rw-` and `uploading` until version 2 lands; the rest as D4.

- [ ] **Step 9: D5, a save in the seconds after the create lands**

```bash
start_step D5; control D5
( while :; do ls "$ROOT" | grep -c '^t2\.txt$'; sleep 0.2; done ) > $EVID/R5-D5-presence.txt 2>&1 & echo $! > $EVID/R5-D5-presence.pid
printf 't2\n' > "$ROOT/t2.txt"; cp $QA_TMP/big-1200m.bin "$ROOT/d5-big.bin"
until [ "$($PSQL -c "SELECT count(*) FROM files WHERE size_bytes = 3 AND NOT is_uploading AND created_at > '$(cat $EVID/R5-D5.start)'")" = "1" ]; do sleep 0.2; done
printf 't2 edited\n' >> "$ROOT/t2.txt"
sleep 100; kill "$(cat $EVID/R5-D5-presence.pid)"; end_step D5
```

Pass: `R5-D5-presence.txt` never reads 0; `ls "$ROOT" | grep -c '^t2 2'` = 0 and one server file for t2; one `provisional id resolved to the server id` line with `request=modify` or `request=deletion_conflicted_create` (record which, and whether the extension's `write options` line in `R5-D5-fp.log` showed the deletion-conflicted bit); the latest version is the appended bytes (`t2\nt2 edited\n`).

- [ ] **Step 10: D6, two saves across a pass boundary**

```bash
cp $QA_TMP/big-250m.bin "$ROOT/d6.bin"; sleep 120                                  # landed and materialized
start_step D6; control D6
F=$(sqlite3 "$STATE_DB" "SELECT file_id FROM files WHERE path = 'd6.bin'")
sample "$ROOT/d6.bin" D6
# Save A two seconds before a pass: read the last pass's time from the log, wait until 28 s after it.
printf 'save A marker...\n' >> "$ROOT/d6.bin"
until [ "$($PSQL -c "SELECT is_uploading FROM files WHERE id = '$F'")" = "t" ]; do sleep 0.5; done
printf 'save B marker...\n' >> "$ROOT/d6.bin"
sleep 150; stop_sample D6; end_step D6
```

Pass: from the first sample whose last 16 bytes are B's marker (`printf 'save B marker...\n' | tail -c 16 | xxd -p`) until B lands, no sample shows A's (`printf 'save A marker...\n' | tail -c 16 | xxd -p`); no fetch of `$F` (both instruments, control passed); status `uploading` until B lands (the fp log shows `m:rw` for `$F` throughout); the server has A then B as consecutive versions.

- [ ] **Step 11: D7, evict and read back**

Finder "Remove Downloads" on `t.txt`, `d4-big.bin` and `d2-fixture.txt` (`fileproviderctl` has no evict on this macOS, `QA/R4-evict.txt`). Before: `shasum -a 256` each into `R5-D7-before.txt`. Then:

```bash
ls -lO "$ROOT/t.txt" "$ROOT/d4-big.bin" "$ROOT/d2-fixture.txt" > $EVID/R5-D7-evicted.txt        # compressed,dataless
shasum -a 256 "$ROOT/t.txt" "$ROOT/d4-big.bin" "$ROOT/d2-fixture.txt" > $EVID/R5-D7-after.txt  # hydrates
diff $EVID/R5-D7-before.txt $EVID/R5-D7-after.txt && echo "read-back identical"
cmp <(cut -d' ' -f1 $EVID/R5-D4-expected.sha) <(grep d4-big $EVID/R5-D7-after.txt | cut -d' ' -f1) && echo "D4 bytes are the appended bytes"
```

Then append to `d6.bin` and at once "Remove Download" on it: `ls -lO "$ROOT/d6.bin"` must not show `dataless` while its save is queued. Pass: as written above.

- [ ] **Step 12: D8, a remote change still reaches the disk**

Upload a new version of `t.txt` from the local web app (it uses the versioned route). Pass, within a pass and its signal: `t.txt`'s `cver` in the fp log changes to a numeric `{cv}`; instrument 1 counts one `fetch-content` for its id; `shasum` of the local file equals the uploaded file's.

- [ ] **Step 13: D8b, a versionless replace from another device, driven by a script** (spec §14 D8b, ruling 3)

The helper, in scratch only (never committed):

```bash
mkdir -p $QA_TMP/d8b-legacy-replace/src
cat > $QA_TMP/d8b-legacy-replace/Cargo.toml <<'TOML'
[package]
name = "d8b-legacy-replace"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
beebeeb-core = { path = "/Users/guuslangelaar/Development/Beebeeb/beebeeb.io/repos/core/beebeeb-core" }
base64 = "0.22"

[workspace]
TOML
cat > $QA_TMP/d8b-legacy-replace/src/main.rs <<'RS'
//! D8b: one legacy chunk, encrypted the way the mobile app's legacy upload sends it
//! (nonce || ciphertext under the file key). Args: master_key_b64 file_id plaintext_in chunk_out.
use base64::{engine::general_purpose::STANDARD, Engine};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let key: [u8; 32] = STANDARD.decode(&args[1]).expect("base64").try_into().expect("32 bytes");
    let master = beebeeb_core::kdf::MasterKey::from_bytes(key);
    let file_key = beebeeb_core::kdf::derive_file_key(&master, args[2].as_bytes());
    let plain = std::fs::read(&args[3]).expect("plaintext");
    let chunk = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, &plain).expect("encrypt");
    std::fs::write(&args[4], chunk).expect("chunk");
}
RS
```

The session (`$TOKEN`, `$MK`) is step 3's. The replace (endpoints as the spec's D8b names them; `S` is `t.txt`'s server id):

```bash
legacy_replace() {   # $1 = server id, $2 = plaintext file
  local S=$1 PLAIN=$2
  local META NAME PARENT SIZE
  META=$(curl -sf -H "Authorization: Bearer $TOKEN" http://localhost:3001/api/v1/files/$S)
  NAME=$(python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["name_encrypted"]))' <<< "$META")
  PARENT=$(python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin).get("parent_id")))' <<< "$META")
  SIZE=$(wc -c < "$PLAIN" | tr -d ' ')
  cargo run --quiet --manifest-path $QA_TMP/d8b-legacy-replace/Cargo.toml -- "$MK" "$S" "$PLAIN" $QA_TMP/chunk0.bin
  curl -sf -X POST http://localhost:3001/api/v1/files/upload/init -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"file_id\":\"$S\",\"name_encrypted\":$NAME,\"parent_id\":$PARENT,\"is_media\":false,\"size_bytes\":$SIZE,\"chunk_count\":1,\"created_at\":null}"
  curl -sf -X PUT http://localhost:3001/api/v1/files/$S/chunks/0 -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/octet-stream' --data-binary @$QA_TMP/chunk0.bin
  curl -sf -X POST http://localhost:3001/api/v1/files/$S/upload/complete -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{}'
}
```

Dry-run first on a throwaway file created in the web app (`d8b-dry.txt`): `legacy_replace <its id> <a plaintext file>`; the dev DB shows it at v+1 with `is_uploading = false`, and the web app opens it with the new text. Then the step:

```bash
start_step D8b
S=$(id_of t.txt $EVID/R5-setup-state.db)
printf 'D8b: replaced from another device\n' > $QA_TMP/d8b.txt
V=$($PSQL -c "SELECT version_number FROM files WHERE id = '$S'")
legacy_replace "$S" $QA_TMP/d8b.txt
$PSQL -c "SELECT version_number, is_uploading FROM files WHERE id = '$S'" > $EVID/R5-D8b-devdb.txt   # V+1, f
sleep 80; end_step D8b
```

Pass: within two passes a `file version raised from the snapshot` line for `$S` (`old_version=$V`, `new_version=$((V+1))`); `t.txt`'s `cver` changes to `{cv}`; instrument 1 counts 1 `fetch-content` for `$S`; `shasum` of the local `t.txt` equals `$QA_TMP/d8b.txt`'s. Never satisfied with the web app, which uses the versioned route.

- [ ] **Step 14: D9, logs**

```bash
grep -E "queued write took over|provisional id resolved|delete of a provisional id|queued write based on the version|file version raised|queue state moved|local landing will be retried|Finder hydrate served|parked with its bytes|upload refused|Finder write refused" $EVID/R5-app-stdout.log > $EVID/R5-D9-lines.txt
grep -cE '\.txt|\.bin|/Users/' $EVID/R5-D9-lines.txt                                   # 0
/usr/bin/log show --start "$(cat $EVID/R5-D1.start)" --predicate 'process == "BeebeebFileProvider" AND eventMessage CONTAINS "write options"' > $EVID/R5-D9-options.txt
```

Pass: every line in `R5-D9-lines.txt` matches a row of spec §11 (message and fields); 0 names or paths. Record every `write options` line and its bitmask; any `FailOnConflict` (bit 1) or immediate-upload (bit 2) bit is written down for spec §16 Q7.

- [ ] **Step 15: D10, record only**

From `R5-D2-fp.log`: after D2's create, did the item for S keep the materialized file (docID unchanged, no `providing` claim before the first save)? `grep -nE "docID|providing" $EVID/R5-D2-fp.log > $EVID/R5-D10.txt`. Write the observation into the task file; it decides spec §16 Q2 (1880's priority).

- [ ] **Step 16: Record, then restore**

Add `## Verification evidence` to the 1873 task file: one line per step (`D1: pass — …`) with its counts and file names, and every control's result. A step that cannot run gets its line amended in place per `.claude/tasks/README.md`, never satisfied badly. Then:

```bash
kill "$(cat $EVID/R5-app.pid)"
launchctl unsetenv BB_API_BASE; launchctl getenv BB_API_BASE        # prints nothing
rm -rf "$QA_TMP"                                                    # the scratch dir only, never $HOME
```

Reinstall whichever app the lead wants on the Mac (`~/bb-qa/1873-r5/Beebeeb-931b885.app` is the one that was there).

---

## Merge and release preconditions

- **Merge** (the lane's branch into `main`) only when: every task's gate is green at the branch head in a fresh tree (Task 13 step 1); D1–D9, D8b included, passed with their evidence and every "no fetch" claim has its positive control in the same folder; the PR body names the split pieces and their task (1879). Recommended before merge, not decided here: a `crypto-security-reviewer` pass on Task 8's queue-served hydrate (`serve_hydrate`, `write_hydrated_from_reader`), because it writes plaintext to a destination that arrives over the socket (the task 1247 hardening).
- **Release:** 1873 is not RELEASED until **task 1887** is fixed: a sign-out by choice purges queued Finder saves (spec §10.5), and 1873 is what makes such saves exist.
- **Release notes (0.8.12):** an authored `RELEASE_NOTES.md` per `docs/RELEASING.md`, and it must state:
  - **the one-time re-download** (spec §10.1): after the update, a materialized file whose old identifier was led by an upload's timestamp, and a materialized server-known file at version 0, is downloaded once more when the system first re-reads it; nothing is lost;
  - the downgrade caveat (spec §5.6 m-4b, §16 Q4): installing an older build while a save is still uploading can put the server's bytes over that save on disk; let saves finish before downgrading;
  - the counted test results (`test result:` lines and the Swift `96 passed`).

## Execution notes for the lead

**Task count: 13.** Twelve lane tasks, one commit each, in spec §15's 4c order; Task 13 is lead-only.

| Task | Commit (spec §15, 4c) | Spec tests | Plan tests | Lib count after |
|---|---|---|---|---|
| 1 | schema, token helpers, one read | T7, T36 | P1 | `L0 + 3` |
| 2 | serialization primitives | T29, T42 | P2, P3 | `L0 + 7` |
| 3 | accept transaction, base mapping | T12, T14, T21, T22, T39, T49, T50 | P5 | `L0 + 15` |
| 4 | Rule 1 reporting | T1–T6, T8, T9, T57, T62, T64, T65 (+ the framing test changed in place) | RF1 | `L0 + 28` |
| 5 | Rule 4: 409 classes, parks, hand-over | T23–T26, T41, T67 (+ the 409 test rewritten) | P8, RF4 | `L0 + 36` |
| 6 | Rule 2 snapshot side | T11, T13, T45–T48, T52, T53 (+ the flag test rewritten) | — | `L0 + 44` |
| 7 | M1 landing transaction | T27, T30, T40, T61 | P9, P10 | `L0 + 50` |
| 8 | Rule 3 | T15–T19, T43, T54, T55, T56′, T58; Swift T20, T44 | RF3 | `L0 + 61`; Swift 96 |
| 9 | Rule 5 | T31, T33, T60 | RF2, RF5 | `L0 + 66` |
| 10 | Rule 6 | T34, T66 | — | `L0 + 68` |
| 11 | Rule 7 | T37 (11 tests), T38 | — | `L0 + 80` |
| 12 | docs | — | — | `L0 + 80` |
| 13 | device D1–D10 (lead) | — | — | — |

T10 (the round-3 old-identifier tests) stays green at every gate. The spec's T28, T32, T51, T56, T59 and T63 moved to 1879 (spec §12) and are not in this plan. The task file for 1879 needs the six pieces added (workspace, lead).

- **One lane, sequential.** Each task consumes the previous one's types and SQL. Suggested model: a strong lane for Tasks 3, 5, 7 and 8 (the transactions and the hand-over); Sonnet suffices for 1, 9, 10, 11 and 12.
- **Batch boundaries for review:** 1–2 (schema, primitives); 3–4 (the token end to end: review the IPC framing reply change with them); 5–7 (queue and landing: the riskiest); 8 (Rust and Swift together; the extension build is part of its gate); 9–11; 12.
- **Where it can go wrong:**
  - Task 2's change of `execute_operation` / `upload_version` signatures touches Keep Mine (`EB:3193-3201`) and the Windows finalization path; their existing tests are the check.
  - Task 3 step 1.1 (switching the Finder tests' entry points): a test missed there keeps exercising the old path and stays green for the wrong reason. Review the list the lane reports.
  - Task 7 folds three earlier mechanisms into one transaction; the `Remove` / `Removed` split must not double-delete or skip the unlink.
  - Task 8's Swift change must keep `check-ipc-timeouts.py`'s call-site text (`contents: contentsURL.flatMap { IPCContentFingerprint.ofFile(at: $0) }`) intact.
  - The concurrency tests (T39, T40, T41, T64, T65) depend on the seams firing once; a seam left armed by a failing test does not leak (each test has its own bridge, and the builder seam is thread-local).
  - Task 13's D3–D6 depend on timing around the 30 s pass; a step that misses its window is rerun, not passed.
- **Decisions for the lead:** Spec issues 1–14 (each resolved here as stated); whether 1879 takes the six split pieces or a new task does; the merge-time security review (recommended above).

### Spec coverage map

| Spec | Task(s) |
|---|---|
| §5.1 token format; §5.2 columns (without the split pieces) | 1 |
| §5.3 predicate, one read, guarded clear | 1, 4 |
| §5.4 states 1–17 | 3 (1–4), 4 (5–13, 15), 7 (14), 6 (8b), 9 (13 presentation) |
| §5.5 our landing moves nothing | 4 (T1, T3), 7 (T40) |
| §5.6 thumbnail safe default; mtime (split); downgrade m-4a, m-4c | 8 (T56′); 3 (P5), 10 (T66) |
| §6.1 rules 1a, 1b, 2, 3, 4, 5, 6a′, 6b; 6a | 3; 6 |
| §6.2 C1 by construction | 3 (T39), 4 (T8, T9) |
| §6.3.1 `init` guard; §6.3.2 versionless ops, fill, raise, request counter; §6.3.3 `base_pending`; §6.3.4 | 3; 6; 6; 6 (T52) |
| §7.1 alias written and swept; §7.2 resolution, gated delete, deletion-conflicted create; §7.3 unknown ids, no item-less replies, the Swift branch; §7.4 fetch from the queue, no status change | 7, 8; 8; 8; 8 |
| §8.1 successor and chain by write id; §8.2–§8.3 no supersede, always wait | 3, 7; 3 (T21, T22) |
| §8.4 immediate parks, 409 classes, the hand-over at the claim and its cost | 5 |
| §8.5 insertion order, restores in content order | 2, 4 (T57) |
| §8.6 rules 1–4, 6 (rule 5 split) | 7 |
| §8.7 S1–S6 | 2 (S2–S4, S6, claim), 3 (S1.1, S5), 4 (S1.9), 6 (S1.5), 7 (S1.2), 8 (S1.8) |
| §8.8 traps: minting keyed on the IPC entries, explicit column lists, options logged | 3, 1, 11 |
| §9 status follows the queue | 7 (landing), 9 |
| §10.1–§10.3 upgrade, earlier-build ops | 10 |
| §10.5 sign-out purge, provisional rows | 4 (T6, T62) |
| §11 logs, M5 | each task's lines; 11 |
| §12 split pieces → 1879 | not implemented here (by ruling 2) |
| §13 gates | every task; 13 step 1 |
| §14 D1–D10 | 13 |
| §15 order and cost | the task order above |
| §17 docs; release notes | 12; merge and release preconditions |
