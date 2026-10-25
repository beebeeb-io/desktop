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
