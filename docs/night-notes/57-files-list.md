# 57 — Files show in their own folder: the snapshot path is not the loss

Date: 2026-11-10 (night wave 3)

## What was checked

A test named `file_and_folder_both_list_after_the_snapshot_path` built one folder
(`Budget 2026`) and one file (`Contract draft.pdf`) through the real snapshot path —
`EngineBridge::prepare_owned_rows` and then `apply_snapshot` — and asserted that
`model::list` on their parent returns both. It was run first, before any product change:

```
date: 2026-11-10T11:30:50+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml file_and_folder_both_list
running 1 test
test drive_v2::tests::file_and_folder_both_list_after_the_snapshot_path ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2669 filtered out; finished in 0.03s
```

The run was already green, so no product code was changed and the test was removed from
the tree (it only restated a path that already passes). After removal, the same module
still passes:

```
date: 2026-11-10T11:31:12+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml drive_v2::tests
running 24 tests
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 2645 filtered out; finished in 0.65s
```

Evidence (absolute paths, outside the repo):
`~/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/files-in-list/green-first-run.txt`
and
`~/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/files-in-list/green-final-tree.txt`.

## Where the loss is

The Files list layer (`drive_v2/model.rs`: `list` → `live_rows` → `file_overview_rows`,
and `apply_snapshot` / `apply_metadata_file_row`) keeps a decryptable folder and file
together. The drop is one layer earlier, in `src-tauri/src/owner_keys.rs`:

- `prepare_owned_rows` (around line 729) does `continue` when a row needs the canonical
  fetch and that fetch is missing, so the row never reaches `apply_snapshot` at all.
- `stored_name` (around line 263) returns `None` when the stored key record cannot be
  resolved for a non-foreign id, so `apply_metadata_file_row` has no name to compose a
  path from.
- `blocked_name` (around line 232) is the "Unavailable file …" label a blocked row gets;
  it is not the cause and must not be renamed or uploaded.

`owner_keys.rs` is out of this slice's scope. The screen is unchanged until that layer is
fixed; slice 63 (preview) parks behind it.
