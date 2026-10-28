# 2190 — a parked stale_base edit on a Mac becomes a reviewable conflict, with the three choices acting on the parked op

**Status:** plan accepted by the lead on 25 Oct 2026 with rulings D1 to D5 (§6). Docs only: no code, no spec text is changed here. Code starts after conflict-2190's Windows fix and PR #142 (Review, the conflict sheet) have merged.
**Date:** 25 Oct 2026
**Task:** 2190 (desktop, P1), follow-up "macOS stale_base becomes a reviewable conflict"; with the macOS fleet task 1683.
**Repo:** desktop (public).
**Written against:** desktop `origin/main` `814bb891` and PR #142's head `704af9bb`. Nothing here was run on a Mac. The Windows fix (`fix/1640-windows-edit-queue`) was not read; §3.1 names where it must agree.
**Findings it builds on:** the task file's 25 Oct findings (lane conflict-2190, red tests on `b89380d0`): on a Mac a Finder edit and a web edit both survive, the edit parks `stale_base` with its bytes kept, and the app offers no recovery.
**Spec:** `docs/specs/2026-10-09-macos-finder-write-versions.md` (§8.4 parked writes) and `docs/specs/2026-10-24-macos-two-surface-app.md` (§7.6, the conflict sheet).

---

## 1. What is there today

| Piece | Where | What it does |
| --- | --- | --- |
| A Finder edit plus a web edit | conflict-2190's pin `task_2190_a_finder_edit_and_a_web_edit_park_as_stale_base_and_never_become_a_conflict` | The Mac edit uploads with base 1, the server answers 409 stale base, and the op **parks**: `state_db.rs` `park_claimed` (~5680) sets `attempts = max_attempts` and `last_error = last_error_class = 'stale_base'`, and `settle_status_after_park_conn` (~1445) flips the file's status to `error` and records a change for the system. The staged copy stays in the container's `finder-writes` folder under the `staged_payloads` journal. Finder keeps showing the Mac's bytes under the held token `b:wW`. |
| How long it stays parked | the write-versions spec §8.4 | Only a NEWER Mac save of the same file takes over its role at its claim; it inherits the stale base, gets the same 409 and parks too. Nothing else ends a park, and a stale base never becomes valid again. |
| What Review shows | `engine_bridge.rs` `version_conflict_feed_from_db` (~5278), `review_entry_for_operation` (~6800), `classify_review_operation` (~6831, the stale_base arm ~6890) | kind `stale_base`, tag "Changed elsewhere", `action: "review_upload"`, `op_id` set. `reviewEntryAction` (#142) shows a button only for `action == "open_conflict"`, so the row has none. |
| What the three choices do | `lib.rs` `resolve_conflict` (~13243), `resolve_keep_mine` (~4781), `resolve_keep_theirs` (~4876), `auto_resolve_keep_both` (~4916) | All three work on **the file at `<sync_root>/<entry.path>`** and on a `FileStatus::Conflict` row: Keep mine stages that file and uploads it with base `None`; Keep theirs hydrates over it; Keep both renames it and hydrates. The preview (`conflict_content_preview`, ~4974) reads the same file. |
| Who sets `Conflict` | nobody (the dead detector, review-142d) | So the choices serve a state the engine never produces. |

## 2. Why the choices cannot just be pointed at a parked row

1. **The Mac's bytes are not at `<sync_root>/<path>`.** They are in the op's staged payload (`operation_queue.payload_path`, journalled in `staged_payloads`). Keep mine would upload the wrong file (or none); Keep both would rename the wrong file.
2. **Base `None` is refused on a Finder write.** Keep mine sends `base_version: None`; the write-versions spec's I2 guard (§6.3, `upload_init_request_for_operation`) parks a File Provider replace without a base as `base_unknown`. A resolution has to send an explicit base: the server's current version.
3. **The row is keyed by `file_id`, the park by `op_id`.** The sheet calls `conflict_content_preview(fileId)` and `resolve_conflict(fileId, choice)`. A file can have a parked op and newer queued saves behind it, so the choice has to name which op it acts on.
4. **Finder has to be told.** A resolution changes what the item is: the held token goes, the content version moves, and the system needs a change record and the item in the tick's signal (as `park_claimed` does). Nothing in the three bridge methods does that for a File Provider item.

## 3. Design

### 3.1 One engine entry point, a platform-neutral core, a macOS seam

`EngineBridge::resolve_parked(op_id, choice) -> Result<ParkedResolution, ResolveError>` in `engine_bridge.rs`, and `parked_conflict_preview(op_id)`. The core reads the parked op and acts in one `StateDb` transaction (the pattern of the claim: an Immediate transaction, and "the op moved since" returns `Moved`, S2). The platform seam is the last step only: macOS records a change and puts the item in the signal. Windows shares the core next (D5), after its fix lands and agrees on the shape of a parked modify.

Preconditions, all checked inside the transaction; a failure is `Moved` and the sheet says "This file changed again. Close this and decide again." and reloads the feed:
- the op exists, kind `upload_version`, `last_error_class = 'stale_base'`, `attempts >= max_attempts`, not claimed, no recorded completion (the exclusion `park_claimed` uses);
- it is the **newest write of the file**: nothing is queued behind it (D2).

### 3.2 The three choices on the parked op

| Choice (sheet label) | Effect on the parked op and its bytes | What Finder and the web see |
| --- | --- | --- |
| **Keep this Mac's** (`local`) | Fetch the server's current version `v` (a failure is an error, nothing changed). Rebase the SAME op: `base_version = v`, `base_object_version_id = current`, `attempts = 0`, `last_error = last_error_class = NULL`, `next_retry_at = now`. The runner uploads it through the normal path and the normal landing (`apply_landing`: token `b:wW` becomes `{cv}`, change recorded, staged copy released). If the server moved again it 409s and parks again: the feed shows it again and nothing is lost. | The web gains a new version on top; the other device's version stays in history. Finder keeps the Mac's bytes and the item settles at the new version. |
| **Keep the other** (`remote`) | **The Mac's edit is never discarded outright (D1).** First its bytes are copied into the 1887 "changes not sent" folder (`kept_unsent_folder`, spec `2026-10-10-macos-sign-out-waiting-writes.md`), and the choice fails with nothing changed if that copy cannot be made. Then, in one transaction: remove the parked op, clear the file's held token through the existing predicate (`UPDATE files SET held_… = NULL WHERE held_write_id = ?`), set the row so Finder re-reads the server's version, record the change and signal. After the commit the staged copy is released (S6, `remove_or_log`). The copy stays in the folder **until the person deletes it**; there is no automatic expiry, and any cleanup policy is a separate decision. | Finder replaces its bytes with the server's on the next fetch. The Mac's edit is a file in the "changes not sent" folder, which Home and Review already list (the 1887 and 1882 kept rows, spec §8.3). |
| **Keep both** (`both`, primary) | In one transaction: turn the parked bytes into a NEW file next to the original: enqueue a create (`upload_file`) in the same parent with a provisional id, named `{stem} ({device}, HH:MM).{ext}` and **unique** (§3.3), whose `payload_path` is the parked op's staged copy (journal ownership moves to the new op); remove the parked modify; clear the held token and re-anchor the original at the server's version as in "Keep the other". The create uses the existing provisional-id machinery (write-versions spec §7: alias, re-key at landing), so name encryption needs no new key logic. | Both files exist on the web and in Finder (the new item arrives through the enumerator after the create lands). The original is the other device's version. |

Invariants, each one a test: no choice releases a staged copy before the commit that stops needing it; no choice runs unless the op is still the newest parked one; `local` and `both` cannot lose the Mac's bytes on any failure (a failed fetch, a failed commit, a refused upload); `remote` removes the staged copy only after the kept copy exists and the transaction has committed; nothing touches `FileStatus::Conflict` rows or the Windows and Linux paths.

### 3.3 The name of the "Keep both" copy is unique (D4)

`{stem} ({device}, HH:MM).{ext}`; if a file of that name already exists in the parent, a counter is added: `{stem} ({device}, HH:MM) 2.{ext}`, then `3`, and so on. The choice never overwrites, and never takes a name that is in use. The check is one pure function over the names that are already there: the parent's rows in `files` (decrypted names, case-insensitive as a Mac is), the queued creates that have not landed (their `target_path`), and the names the same resolution has already taken. Not covered, and said so in the sheet's tests rather than hidden: the server cannot enforce uniqueness (it holds encrypted names), so two devices that resolve at the same minute can still each create the same name; they are two files with two ids and nothing is overwritten.

### 3.4 Feed, commands and sheet

- **Feed:** on a Mac a parked `stale_base` upload gets `action: "open_conflict"` (Review then shows "Decide…" through the existing `reviewEntryAction`); the kind stays `stale_base` ("Changed elsewhere") and `op_id` stays. Other park reasons (`base_unknown`, `payload_missing`, `predecessor_*`, `rekey_failed`, `permission_denied`) keep today's rows (D3).
- **Commands:** `parked_conflict_preview(op_id)` and `resolve_parked_conflict(op_id, choice)`, session-admitted with `session_command!`, which moves the census count in `tests/windows_session_wiring.rs` by 2, deliberately. `resolve_conflict` and `conflict_content_preview` stay for rows that really are `Conflict` (Windows and Linux, and the 24 h sweep).
- **Preview:** the local side from the staged payload (bounded read with the same caps as `local_content_preview`: stat first, cap plus one byte, text only when small); the remote side from the server's current version (`remote_content_preview`, reused). Neither moves any state.
- **Sheet (#142's `ConflictSheet.tsx`):** when `entry.op_id` is set it uses the two new commands, otherwise the old ones. Same three buttons, same second press on "Keep the other". **Copy that must change with D1:** the second press's question says "Replace this Mac’s version? Your changes here are discarded." (`CONFLICT_REPLACE_QUESTION`, #142's `reviewModel.ts`), which is no longer true. Confirmed by the lead on 25 Oct 2026: "Replace this Mac’s version? A copy of your changes stays in the Changes not sent folder." It is true only because D1 copies first and fails closed, so it is pinned to the code path that does the copy (§4 step 3). #142’s current sheet (the dormant `sync_root` flow) keeps its earlier claim-free "Use the other version here?": it keeps no copy, so it cannot make that claim. When the sheet is routed through `resolve_parked`, the new sentence replaces it there, and the spec records that in the same commit. One new failure sentence for `Moved`.

## 4. Order of work (red first, each step its own commit)

1. **Red engine tests** on the 2190 pin's fixture (a parked stale_base op with staged bytes and a server at v2): for each choice, the state after (op, held token, staged copy, kept copy, change row, server versions), plus the `Moved` cases (op gone, a newer save queued behind it, claimed, a completion recorded). The 2190 pin flips from "never becomes a conflict" to "parks and is decidable".
2. **Unique-name tests** (D4), a pure function: no clash gives the base name; the base name taken gives ` 2`; ` 2` taken gives ` 3`; a queued create that has not landed counts; no extension; a dotfile; a stem with parentheses; two names taken in the same resolution; case-insensitive clash.
3. **Kept-copy tests** (D1): the copy exists and holds the bytes before the op is removed; a failed copy changes nothing; the staged copy is released only after the commit; the kept copy is still there after the app restarts and is never swept. Plus the copy-claim pin: a test that the kept copy exists on disk before the question's action commits, and one that fails if the sentence in the sheet is used on any path that does not copy first.
4. Core: `resolve_parked` and the preview, each choice in its own commit.
5. Feed `action`, the commands, the census and wiring pins.
6. TypeScript: the sheet routes by `op_id`, the `Moved` sentence, the confirmed second-press copy, tests in `conflictSheet.test.tsx`, a render scenario ("Decide…" on a "Changed elsewhere" row, the three choices, the second press).
7. A mutation table for every rule above, and counts per binary, as always.

Dependencies: code waits for conflict-2190's Windows fix (a parked modify must have the same shape on both sides) and for #142. Size: roughly 2 to 3 days for the Rust part with its tests, 1 day for the sheet, plus the device pass.

## 5. Not covered

- The dead detector (`is_conflict` compares a hash with itself) and the 24 h auto keep-both for `Conflict` rows: the full rewrite stays its own follow-up. This plan uses the parked op as the conflict, not `FileStatus::Conflict`.
- Linux (no local-edit upload path) and Windows (the P0 fix, then D5).
- Folders, deletes and renames that conflict: only a content edit parks `stale_base`.
- A cleanup policy for the "changes not sent" folder (D1: a separate decision).
- Device rungs for the lead's pass, not run: edit a file on the Mac and on the web before syncing, then Review shows one "Changed elsewhere" row; each choice in turn, then Finder, the web (versions and files) and a restart of the app must agree; the kept copy exists after "Keep the other"; "Keep both" twice in the same minute gives ` 2`; the same with a newer Mac save queued behind (the choice is refused); sign-out with a parked op (1887's unsent-changes sheet already counts parked ops, to be re-checked); a full disk while copying for D1 and while staging for Keep both.

## 6. Rulings (lead, 25 Oct 2026)

| # | Question | Ruling |
| --- | --- | --- |
| D1 | "Keep the other" discards the Mac's edit. | Never discards it outright: a copy goes to the 1887 "changes not sent" folder and stays **until the person deletes it**, with no automatic 24 h expiry. Any cleanup policy is a separate decision. (§3.2; the sheet's second-press copy changes with it, §3.4.) |
| D1 copy | The second-press sentence on a parked conflict. | "Replace this Mac’s version? A copy of your changes stays in the Changes not sent folder." Accepted, on the condition that it is pinned to the code path that copies, with a test that the copy exists before the action commits. #142’s dormant `sync_root` sheet keeps "Use the other version here?" until it is routed through `resolve_parked`; the spec change lands in the same commit as that routing. |
| D2 | A newer Mac save is queued behind the parked op. | Accepted: refuse resolving a parked op when a newer save is queued behind it, and show the newest. |
| D3 | `base_unknown` parks. | Accepted: unchanged in this slice. |
| D4 | The name of the "Keep both" copy. | The format is accepted, **and it must be unique**: if `{stem} ({device}, HH:MM).{ext}` exists, add a counter. Never overwrite (the #142 P2-B lesson). Test it. (§3.3.) |
| D5 | Windows shares `resolve_parked`. | Yes, next, after the P0 fix lands. |

## 7. Addendum (lane conflict-2190, 28 Oct 2026): the detector, and this PR's scope

Written before the code of `fix/2190-conflict-detector`, against desktop `main` `f6614694`. The lead's brief for follow-ups 1 and 2 is the source; the four decisions below were sent to the lead before any code.

### 7.1 Decisions

- **E1. The parked op is the conflict, on every platform.**
  - `FileStatus::Conflict` stays unset, so the 24 h auto Keep Both sweep and the per-file conflict windows stay dormant, as they are today.
  - A row with a queued or parked write is `Uploading` or `Error`, never `Local`, so the tick's Local branch only sees rows with no pending write. The authoritative signal is the server's 409 on the write's base, which parks with both versions kept.
- **E2. `content_hash` gets no production writer.** On macOS it is part of the File Provider content version (`item_content_version`). A writer would change every item's content version, so Finder would see every file as changed. The detector reads 1640's agreed content instead: `local_hash` at `local_base_version`.
- **E3. The first-sight base is never invented.** `apply_metadata_file_row` no longer sets `local_base_version = current_version` for a `Local` row at 0. A base of 0 stays unknown, and an edit with an unknown base is held, not sent (task 2229).
- **E4. Windows resolution (D5) is the next PR**, after 2191 S2 merges. "Keep the other" and "Keep both" on a sync-folder file must replace the edited disk file after the copy is made, which needs a hydrate that overrides 1640's unsent-edit refusal: the same code 2191 S2 changes. In this PR, Windows parked rows keep today's row.

### 7.2 The detector: decision table (`process_metadata_row`, the `Local` branch)

Inputs: `B` = the edit's base as `sync_folder_content` gives it (`local_hash` at `local_base_version`, or the fetched candidate before it is promoted; corrected on 28 Oct: the first draft read `local_base_version` alone, which is 0 until the candidate is promoted), else `local_base_version` (0 = unknown); `V` = the server version on the pulled row (`current_version`, `version`, `version_number`); `L` = the device's bytes.
- On a sync-folder file, `L` is `sync_folder_content` (Windows, and every test build).
- Elsewhere a `Local` row has no pending write, so `L` = agreed.

| # | L | V against B | Action |
|---|---|---|---|
| 1 | agreed (or unknown) | any | Re-anchor, as today: `remote_updated_at`, `size_bytes` and `modified_at` from the server, then the metadata write. `local_base_version` is unchanged (E3). |
| 2 | changed (an edit the scan has not queued yet) | `V > B > 0` | ~~A concurrent edit. The metadata write still runs (the server's version is recorded), but `size_bytes`, `modified_at` and `remote_updated_at` keep describing the device's bytes, and `B` is kept.~~ **Corrected (lane, 28 Oct, first test run):** the metadata write (`apply_metadata_file_row`) itself writes the server's size and stamps, so a concurrent edit is written to the database exactly as row 1 is, and keeping them would be cosmetic: the pull resizes no placeholder (`resize_placeholder` runs only in the fetch callback). What the row keeps is the edit's base `B`, which the pull never touches. The scan then queues the edit on base `B`, the server refuses it as stale, and it parks: a reviewable conflict (§3). No `Conflict` status, and no `ConflictDetected`. `is_conflict` is true here, and only here, which the tick logs. |
| 3 | changed | `V <= B`, or `B = 0` | Our own edit, not sent yet, or an unknown base: re-anchor as row 1 (the edit is queued on `B`, or held by task 2229). |
| 4 | queued | — | Unreachable: such a row is `Uploading` (branch 3, unchanged). |

~~`is_conflict` is called with `local` = `"changed"` or the agreed hash, `base` = the agreed hash, and `remote` = `"v{V}"` when `V > B`, else the agreed hash. So it is true exactly in row 2.~~

**Corrected after review-169 #7 (lane, 28 Oct):** the pull reads no file and calls no `is_conflict`. Rows 1–3 all take row 1's action, so telling them apart had one effect: a log line. Reading `L` cost a file hash on a cold cache during the pull. The branch was also gated `cfg!(any(target_os = "windows", test))`, so the macOS tests ran a read that macOS production never takes. What row 2 relies on is unchanged and still pinned (`task_2190_fu1_a_concurrent_edit_the_scan_has_not_queued_keeps_its_base_and_parks`, which also asserts 0 file reads in the pull): the pull never touches `B`, so the scan queues the edit on `B`, the server refuses it as stale, and the parked op is the conflict (E1). `conflict::is_conflict` stays for its own tests and a `Conflict` row's three-way rule.

### 7.3 This PR (macOS)

- §3's core: `resolve_parked` and its preview, the three choices, D1 (the kept copy), D2 (newest write only) and D4 (a unique name).
- The feed gives `action: "open_conflict"` to a parked `stale_base` Finder write (`write_id` set) that the core would accept. It uses one predicate, so the feed never offers a choice the core refuses.
- UI: main's review page and `ConflictWindow` route by `op_id`. #142's `ConflictSheet` is not on main, so the sheet change in §3.4 waits for it. The second-press copy in D1 applies to the window's "Keep theirs" on a parked row only.

### 7.4 Re-review of review-169 (lane, 28 Oct): what a decision is made against

- **The version shown (F1).** The window decides against the version of the bytes it showed: the `download_version.version_number` of the metadata the preview's download ran on (after a restart, the fresh metadata's), as `do_hydrate_versioned` reads it for 1640. It is never the mirror read after the download, which the pull can raise to a version nobody saw. Metadata that names no version (a legacy file, or a failed fetch) gets the mirror as it was before the fetch. That is never newer than the bytes, so at worst the choice is `Moved`, or the write goes on an older base and the server refuses it as stale.
- **A trashing file is not decidable (F2).** The one predicate also requires the file's row not to be `trashing`. A trash from another device, of the file or of a folder it is in, marks the row `trashing` and queues nothing. A file deleted for good has no row.
- **The "Keep both" name fits (F3).** At most 255 bytes, built as a kept file's name is (`kept_names::with_suffix`): the stem is cut at a character boundary, never the suffix, the counter or the extension. **New number (lane):** the device name is cut to 64 bytes first (`KEEP_BOTH_DEVICE_MAX_BYTES`), so the suffix always leaves the stem room. A host name's label is at most 63 characters, so a Mac's own one-label name (`MacBook-Pro`) is never cut; only a long multi-label name is.
- ~~**Not covered:** a LOCAL trash of the parent folder while the window is open. `queue_finder_delete` marks only the folder row until the server's echo marks its children, and D2 sees no later op of the file itself. The echo closes it.~~ **Covered (the lead, 28 Oct):** the gap is closed in the same predicate.
- **No folder above is being trashed or moved.** The predicate walks up `parent_id` (a recursive CTE, one primary-key lookup per level). Any folder above that is `trashing`, or that has a queued `trash_file`, `move_file` or `rename_file`, makes every choice `Moved`. A rename counts as a move, as D2 treats it for the file itself. A local trash marks only the folder's row and queues on the folder; a local move or rename queues on the folder. The walk follows `parent_id`, while the pull's mark of a remote trash follows path prefixes. **The walk's depth cap (lane):** 512 levels. macOS paths are at most 1024 bytes (`PATH_MAX`) and every level takes at least two (`a/`), so no real file nests deeper. A chain that reaches the cap (a cycle in the rows) is not decidable.
