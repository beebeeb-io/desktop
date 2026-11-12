# 58 — A folder and a stale row can leave

Date: 2026-11-10 (night wave 3)

## What was checked

Two tests in `src-tauri/src/drive_v2/absent_row.rs`:

- `known_folder_is_trashable`: a folder row the server knows (`remote_updated_at`
  not 0, status `Local`, own namespace) passes `check_trashable`. This is a
  **regression guard**, not the red-first proof: `check_trashable` already
  returned `Ok` for that row on the base tree, so it guards existing behaviour.
- `absent_row_leaves`: a row whose id is absent from the snapshot just applied
  is not returned by `model::list` afterwards, including a row whose stored name
  is the engine's blocked label (`Unavailable file ` plus 16 hex digits), and
  the terminal-failed operation that named it is gone from `operation_queue`
  (so `waiting_rows` no longer returns it). This test carries the red-first
  evidence for the slice.

`absent_row_leaves` was red first, before the drop was added (the probe made
`drop_absent_rows` return straight after `prune_absent`, the pre-fix behaviour):

```
date: 2026-11-10T13:33:43+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml absent_row_leaves
test drive_v2::absent_row::absent_row_leaves ... FAILED
thread 'drive_v2::absent_row::absent_row_leaves' (3648) panicked at src/drive_v2/absent_row.rs:105:5:
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2670 filtered out; finished in 0.01s
```

After the fix:

```
date: 2026-11-10T13:33:06+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml known_folder_is_trashable
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2670 filtered out

date: 2026-11-10T13:34:10+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml absent_row_leaves
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2670 filtered out

date: 2026-11-10T13:35:21+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml prune_absent
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 2659 filtered out
```

Evidence (absolute paths, outside the repo):
`~/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/stale-row/red.txt`
and `green.txt`.

## Where the stale row was kept

`check_trashable` (`drive_v2/commands.rs`) refuses a row with
`remote_updated_at == 0` as `Conflict`, so a failed local row cannot be trashed.
That refusal is unchanged: the row must leave the list instead.

`StateDb::prune_absent` (`state_db/removals.rs`) keeps a row whose
`operation_queue` still names it, including an op that has already failed
terminally (`attempts >= max_attempts`, not paused). A failed upload therefore
kept its row forever, and the row was absent from every snapshot.

The drop is `model::drop_absent_rows` (`drive_v2/model.rs`), called from
`syncflow::apply_snapshot` (`engine_bridge/syncflow.rs`) in place of the bare
`prune_absent` call. It first runs `prune_absent` (unchanged) for the rows with
no op, then drops a row whose every queued op is a terminal failure and that
passes the same guards `prune_absent` applies: own namespace, not
`Uploading`/`Trashing`, `remote_updated_at < snapshot_fetched_at`, parent not
`Trashing`, and never on an empty snapshot. A paused op (waiting for auth or a
replaced key) or an op with attempts left still protects its row.

The row's `operation_queue` entry leaves with it: `StateDb::delete_file_subtree`
(`state_db/removals.rs`) now deletes the removed file's `operation_queue` and
`upload_resume` rows in the same transaction that deletes the `files` row. Before,
the terminal-failed op stayed behind, still returned by `waiting_rows`, and named
a row that no longer existed — the invariant the reconcile path enforces
(`reconcile.rs`). `absent_row_leaves` asserts the op is gone.

## The label written up as a file name

`blocked_name` (`owner_keys.rs`) is the label. Two callers turn it into a name:

- `apply_metadata_file_row` (`engine_bridge/meta_2.rs`, around line 418) uses
  `owner_keys::stored_name` as the row's leaf, so a blocked row's stored path
  becomes `Unavailable file <hex>`. That is the row the person saw.
- `request_row_display_name` (`engine_bridge/upload.rs`, around line 740) uses
  the same `stored_name` as an upload name; `blocked_name` passes
  `safe_display_name`, so an edited request row can be uploaded under the label.

Neither is changed here: `owner_keys.rs` is out of this slice, and the label is
not renamed or uploaded by this change. Named for the owner of that layer.

## Not done

- `check_trashable` still refuses `remote_updated_at == 0`; a local-only row is
  not made trashable.
- The Uploading and Conflict refusals and the Backups protection are unchanged.
- `cargo clippy -- -D warnings` is not clean on the base tree (224 pre-existing
  errors, none in the files this slice touched).

## Round 2 review

- P1 (`syncflow.rs:76`): the review line is a placeholder (`P1 ...`), so there is
  no finding text to act on. The only change at that line is the mechanical
  rustfmt pass the pre-commit hook requires: the base `syncflow.rs` is not
  rustfmt-clean, and `rustfmt-staged.sh` refuses any commit that stages it
  unformatted. Reverting the pass makes the commit impossible, so it stays; the
  functional change is the one-line `drop_absent_rows` call in `apply_snapshot`.
- P2 (`absent_row.rs:46`): `known_folder_is_trashable` was never red on the base
  tree; it is labelled a regression guard above, and `absent_row_leaves` carries
  the red-first proof.
- P2 (`model.rs:350`): the dropped row's `operation_queue` entry now leaves in
  the same transaction (`StateDb::delete_file_subtree`), and `absent_row_leaves`
  asserts it. Red probe (the `drop_ops` calls commented out): 0 passed / 1 failed
  at 2026-11-10T14:27:43+01:00, `the dropped row's operation must be gone`.
