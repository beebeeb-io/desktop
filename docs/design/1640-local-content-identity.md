# 1640: local content identity (Windows sync folder)

Status: the design of record for PR #151 from round 3 (review-151c) on. The code is checked
against this document. Task 1640; the follow-ups are task 2199; the files synced before this
build are task 2191, which has its own spec.

## The problem in one paragraph

Windows Cloud Files fetches a placeholder's bytes on demand, often a range at a time, and the
web can save a new version between two fetches. An edit made in the sync folder is sent with a
**base**, the version its bytes are based on. The server refuses an edit whose base is not its
current version (`stale_base`, a 409), so a base that is too LOW is safe: the edit parks with its
bytes. A base that is too HIGH is a silent overwrite: the edit is accepted over web changes it
never saw. Before this PR, Windows sync-folder edits were never sent at all (1640). This PR
sends them, so it must know, for every file, which version(s) its bytes on disk come from.

## The state

Each `files` row carries two descriptions of the bytes at its place on disk.

| Column | Meaning |
|---|---|
| `local_hash`, `local_base_version` | **Agreed content (baseline).** The bytes on disk are exactly version `local_base_version`'s whole plaintext, whose SHA-256 is `local_hash`. `local_hash IS NULL`: no baseline. |
| `fetched_version`, `fetched_hash` | **Fetch candidate.** What fetches put on disk when there is no baseline. |

`fetched_version` values:

| Value | Name | Meaning |
|---|---|---|
| `-1` | unknown, pristine | Bytes may be on disk; nobody can name their version. The column default (`state_db.rs:2285`), so every row from before this build and every new row starts here. |
| `-2` | unknown, fetched into | As `-1`, and a fetch (or a download that named no version) wrote into it since. Only `-1` may ever become `0`. |
| `0` | empty | No fetched bytes on disk (the placeholder was created where nothing was), or the baseline describes them all. |
| `v > 0`, hash set | candidate, one version | Every fetched byte on disk is version `v`'s; `fetched_hash` is the SHA-256 of all of version `v`'s plaintext. |
| `v > 0`, hash NULL | candidate, mixed | The fetched bytes come from more than one version (or may have changed in part); `v` is the OLDEST. |

What the content means for an edit (`sync_folder_content`, `engine_bridge.rs:5654`), first match wins:

1. an upload of the file is queued → **Queued**;
2. the platform says in sync → **Agreed**;
3. a baseline → bytes equal `local_hash` ? **Agreed** : **Changed { base: `local_base_version` }**;
4. a one-version candidate → bytes equal `fetched_hash` ? **Agreed** : **Changed { base: v }**;
5. a mixed candidate → **Changed { base: v }**, never Agreed;
6. otherwise (unknown `-1`/`-2`, or empty with no baseline) → **Unknown**: never agreed, never sent, never freed.

## The invariants

- **I1. No baseline unless the disk is one version's whole plaintext.** `local_hash` is written only
  right after the whole file was written by this engine, or after the bytes were hashed under an
  exclusive handle and equal a one-version candidate.
- **I2. A Changed base is never above the bytes' true version.** Every byte on disk comes from a
  version `>=` the recorded one. The candidate only ever moves DOWN or becomes mixed; only a
  write of the whole file replaces it. Nothing resets it to `0` while bytes may remain.
- **I3. An unsent edit is never discarded.** Agreed is the only state in which the engine may
  free, overwrite or remove the bytes without keeping a copy (Free up space, the dehydrate
  guards, a hydrate into the sync folder, a remote deletion).
- **I4. Every fetch is recorded before its bytes are transferred**, so a transfer that fails or is
  cut off midway is still counted (it can only make the record lower or mixed).

Why a dehydration never needs a reset: it only REMOVES bytes. The bytes that remain still come
from the recorded versions, so the record stays true. Forgetting it is what was wrong (round 3, F1).

## The transition table

| # | Event | Change | Where |
|---|---|---|---|
| T1 | Row created (server tree, local create, migration) | default `-1` | `state_db.rs:2285` |
| T2 | `CfCreatePlaceholders` really created the placeholder (not ALREADY_EXISTS) | `-1` → `0`; only while still `-1` and no baseline | `windows_cf/mod.rs:1586`, `state_db.rs:3731` |
| T3 | A fetch (range or whole) is about to transfer version `v` (hash `h`) | baseline folded in first (T6's merge); then `0` → `(v, h)`; `(v, h)` stays; another version → `(min, NULL)`; `-1`/`-2` → `-2`. `local_hash` → NULL | `engine_bridge.rs:4375`, `state_db.rs:3694` |
| T4 | A fetch's download named no version | content unknown: `-2`, `local_hash` NULL | `engine_bridge.rs:4267` (and the same in `hydrate_file_to_memory`), `state_db.rs:3717` |
| T5 | A WHOLE-file fetch's transfer succeeded | baseline `(v, h)`, candidate `0` | `windows_cf/callbacks.rs:299`, `engine_bridge.rs:4420` |
| T5' | A fetch's transfer failed, was cut off, or hit a size mismatch | nothing more (T3 already recorded it) | `windows_cf/callbacks.rs:309`, `windows_cf/callbacks.rs:243` |
| T6 | The row goes `cloud_only` from any status (a tick's decode of a partial placeholder, Explorer's or Storage Sense's dehydration through the reconcile, the engine's Free up space and selective-sync dehydrate, a cache eviction, an untrash) | **fold**: the baseline joins the candidate (`0` → `(base, local_hash)`; same version and hash stays; otherwise `(min, NULL)`; a baseline with no version → `-1`); `local_hash` → NULL. Never a wipe. | trigger `state_db.rs:2400` |
| T7 | The size resolution: the decrypted size differs from the placeholder's, so it is resized without dehydrating | **poison**: fold, then `fetched_hash` NULL; the version is kept | `windows_cf/callbacks.rs:368`, `state_db.rs:3757` |
| T8 | The startup repair: a row left `downloading` by a stop goes `cloud_only` | T6, then poison (same transaction) | `state_db.rs:3867` |
| T9 | A remote move or rename (the row's path changes; the bytes do not) | candidate poisoned (`fetched_hash` NULL, version kept); baseline kept | `engine_bridge.rs:7996` |
| T10 | `hydrate_file` (Keep Theirs, Keep Both; on Windows nothing else) wrote the whole file by one atomic rename | baseline `(v, h)`, candidate `0`; no version known → T4 | `engine_bridge.rs:4266` |
| T11 | Promotion: the platform says in sync with no modified data, the file is wholly on disk, its bytes hashed under one exclusive handle equal a one-version candidate, and the row is still `Local` with that candidate | baseline `(v, h)`, candidate `0` | `windows_cf/placeholders.rs:836`, `state_db.rs:3776` |
| T12 | A landing that hashed the payload it sent | baseline (produced version, payload hash), candidate `0` | `state_db.rs:4598` |
| T12' | A landing that could not hash its payload | baseline version moves, previous hash kept (reads as Changed, never Agreed) | `state_db.rs:4594` |
| T13 | Sign-out | every row goes | `state_db.rs:5315`, `state_db.rs:5507` |
| — | Revert (restore a version) | nothing: only the server changes; a later fetch records anew | `state_db.rs:4125` |
| — | A dehydration of any kind | nothing beyond T6 (see "Why a dehydration never needs a reset") | — |

**No transition resets a candidate to `0` except T5, T10, T11, T12, whole writes that replace every byte.**

### Not done, and why (task 2199)

- **(a) A wipe to `0` after the engine's own `CfDehydratePlaceholder`.** The design approved in
  chat had it, inside the `CfOpenFileWithOplock(EXCLUSIVE)` window. It is NOT implemented. That
  handle carries an RH oplock: a new opener breaks it and proceeds, so between a successful
  dehydrate and the DB write an app can open the file, and its fetch records (T3) and transfers.
  The wipe would then erase that record while its bytes land. Closing it needs a content
  sequence column (wipe only if unchanged since before the dehydrate).
- **(b) A wipe on a reconcile that sees `OnDiskDataSize == 0`.** T3 records before the transfer,
  so a reconcile can read "nothing on disk" while a recorded fetch's bytes are on their way.
- **Cost of both, accepted:** after any Free up space (the app's, Explorer's, Storage Sense), a
  RANGE re-fetch at a NEW version makes the candidate mixed, so an edit is sent on the old
  version and parks (a false park, safe; a parked Windows edit has no UI yet, P2-1). A WHOLE
  re-fetch gives fresh agreed content (T5). Files of unknown content stay Unknown until a whole
  fetch, a `hydrate_file`, or task 2191.

## Files kept from a remote deletion (rounds 2 and 3)

A file or folder the server no longer lists (a web trash, an emptied trash, a snapshot prune) is
removed only when nothing unsent is lost (I3), `remove_pruned_entry` `engine_bridge.rs:8472`:

| Platform view | Bytes | Outcome | Entry (`kept_copies.reason`) |
|---|---|---|---|
| In sync, no modified data | — | removed | none |
| Nothing on disk (`OnDiskDataSize == 0` AND `ModifiedDataSize == 0`) | — | removed unread | none |
| Wholly on disk | = baseline or one-version candidate | removed | none |
| Wholly on disk | other | copied as "<name> (this PC)" (temp file, then a no-replace move) into the nearest folder that is neither removed nor trashed, then the original removed | `copy` |
| Partly on disk | — | kept in place, never read in the tick | `partly_local`, dismissable (F7) |
| Removal or read refused (open) | — | kept, retried | `in_use` |
| Copy failed | — | kept in place, retried | `copy_failed` |
| A folder that still holds something | — | kept, recorded in `removed_folders` | none |

- A later remote op skips a file whose entry still waits (F4), `engine_bridge.rs:8710`.
- The retry, every tick (`retry_kept_originals`, `engine_bridge.rs:8769`):
  - it leaves a place that belongs to a file the server lists again: a row that is not the deleted file's own `Trashing` row (F2, `engine_bridge.rs:8795`);
  - it registers its own delete with the watcher, so no trash is queued;
  - it updates the entry where it is (F6), and an entry with nothing kept goes;
  - it removes emptied kept folders (`engine_bridge.rs:8846`).
- The scan never creates a kept folder on the server again while it holds only waiting originals,
  kept folders and ignored names (F3, `engine_bridge.rs:3588`,
  `engine_bridge.rs:8878`); once it holds something new, it is wanted again.

## Which tests and mutations pin each row

All in `src-tauri/src/engine_bridge.rs` tests unless noted. "Mutation" names the deliberate break
that turned the test red at its own assertion (evidence: the PR body and the gate logs).

| Rows | Test | Mutation |
|---|---|---|
| T1, T2, T3 (-1/-2) | `regression_1640_a_file_of_unknown_content_stays_unknown_through_fetches` | default `0`; the empty mark unconditional; a `-2` marked empty (`fetched_version < 0`) |
| T3 merge, T3 fold | `regression_1640_a_range_fetch_candidate_merges_by_version` | no fold on a fetch; a mixed candidate keeps its hash; MAX for MIN |
| T3 + T6 with ticks, a user write to a partial file | `regression_1640_a_tick_between_range_fetches_never_raises_the_base` | the trigger wipes on cloud-only |
| T3, T5 (Windows) | `regression_1640_only_a_whole_file_fetch_records_a_baseline` | the baseline written before the transfer (native) |
| T3 + T6 + web save, end to end (Windows) | `regression_1640_range_fetches_across_a_tick_and_a_web_save_never_replace_the_web_version` | the baseline written before the transfer (native) |
| T7, T8 | `regression_1640_a_resize_or_a_cut_off_download_makes_the_content_mixed_never_resets_it` | poison resets; the repair not poisoned |
| T6, T9 | `regression_1640_dehydrating_or_moving_a_file_keeps_what_it_held` | the trigger wipes |
| T11 | `regression_1640_a_candidate_is_promoted_only_when_the_bytes_are_its_own` | promotion without the hash compare |
| T12 | `regression_1640_a_landing_replaces_the_candidate` | the landing keeps the candidate |
| T12, disk changed after staging | `regression_1640_a_landing_whose_disk_changed_after_staging_is_not_agreed` | none: it pins what the round-1 payload hash means (the sent copy, never the disk) |
| T12' | `regression_1640_a_landing_that_cannot_hash_the_sent_copy_keeps_the_previous_baseline` | (round 1) |
| sync_folder_content 4, 5 | `regression_1640_a_candidate_decides_like_a_baseline`, `regression_1640_an_edit_before_promotion_is_sent_on_the_candidates_version` | candidate always Agreed |
| F2 | `regression_1640_a_retry_never_removes_a_file_the_server_lists_again` | no live-row check |
| F3 | `regression_1640_a_folder_deleted_elsewhere_is_not_created_again_for_what_it_keeps`, `..._with_something_new_is_created_again` | the folder created again; the retry's copy without the removed set |
| F4 | `regression_1640_a_second_remote_op_copies_nothing_again` | the second op copies |
| F5, F6 | `regression_1640_a_kept_file_says_why_and_is_resolved_where_it_is`, `regression_1640_a_file_partly_on_disk_is_listed_once_and_resolved_later` | one text for every reason; a new entry instead of an update |
| F7 | `regression_1640_a_file_kept_partly_on_disk_can_be_dismissed` | not dismissable |

## Platform premises not verified here (the native rung)

Everything above runs as unit tests on macOS and natively on Windows with cargo (no sync root is
registered: that is prohibited in the lanes). These premises are therefore assumptions until a
run on a registered sync root on the Windows PC:

1. A partly hydrated placeholder carries `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`; a wholly
   hydrated one does not (`fully_on_disk`, the decode).
2. The platform clears a placeholder's in-sync state when its bytes change, and the engine's
   `CfSetInSyncState` is refused or cleared on a placeholder with modified data.
3. `CfGetPlaceholderInfo` reports `OnDiskDataSize` / `ModifiedDataSize` as documented for full,
   partial and dirty partial placeholders.
4. A placeholder opened with `share_mode(0)` and read access cannot be opened by another process
   for writing while the handle is held (promotion, the stamp).
5. `CfCreatePlaceholders` returns `ERROR_ALREADY_EXISTS` for an existing entry and creates nothing
   (T2's premise: "created" means nothing was there).
6. A FETCH_DATA callback runs only while the requesting opener holds a handle; a cancelled fetch
   delivers no further transfer.
7. `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` refuses an existing target inside a sync root.
8. Office's save sequence (temp file, rename over) leaves a plain file holding the new bytes.
9. Under `CF_HYDRATION_POLICY_PARTIAL`, an app can write into a placeholder that is only partly on
   disk (a "dirty partial" placeholder) without the platform hydrating it first.

Rung: on a registered sync root, (a) Free up space on a really edited file keeps it, (b) a
range-fetched Office file edited and saved is sent on the right version, (c) a web save between
two range fetches, then an edit, parks.

## Residuals (known, not handled here)

- After any Free up space, a RANGE re-fetch at a new version parks an edit (see "Not done").
- The edit detector classifies `Local` rows only. A tick reads a partly hydrated placeholder as
  cloud-only, so a person's write into one is not sent until the file is wholly on disk (the
  decode then reads it `Local` and a later scan sees it). Meanwhile it is kept: it is not Agreed,
  so nothing frees, overwrites or removes it. Whether such a write exists without a hydration
  first is premise 9.
- A write that keeps a file's size and sets its modification time back is invisible to the hash
  cache until either changes.
- A remote move or rename does not move the file on disk (pre-existing on Windows).

## History (claims corrected, left visible)

- Round 2 said a candidate's base "can only be too low" and that a cloud-only row "forgets the
  agreed content and the candidate". The forgetting was the bug: a tick reads a partial
  placeholder as cloud-only, the candidate was wiped while ranges stayed, and the next fetch
  started a fresh candidate above them (review-151c F1). Replaced by T6's fold.
- The round-3 design sent in chat had a wipe after the engine's dehydrate (a); it is not
  implemented (see "Not done").
- A whole fetch used to record the baseline before its transfer; replaced by T3 + T5.
