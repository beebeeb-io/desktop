# 65 — The file row survives the key walk (fix proven, commit blocked by the pre-commit gate)

Date: 2026-11-11 (night wave)

## What was wrong

`EngineBridge::prepare_owned_rows` (`src-tauri/src/owner_keys.rs`) drops a row
before `apply_snapshot` ever sees it when the row's `name_encrypted` does not
decrypt with the loaded master key and the canonical `GET /api/v1/files/<id>`
answers 404. The `continue` at the old line 729 treated "the canonical fetch
raced deletion" as proof the item was gone, but the canonical fetch was the
only reader that could have named the row — a 404 there is not proof the
snapshot's own row is obsolete. The file vanished from the folder listing while
its folder stayed.

## Red first

Test: `owner_keys::tests::lost_row::a_file_row_with_an_unreadable_name_survives_the_key_walk`

It builds one folder row (name readable with the loaded master key `[7; 32]`)
and one file row whose `name_encrypted` is a real envelope sealed under a
different master key (`[9; 32]`), so the loaded key cannot open it. The
canonical fetch answers 404. It then drives the real path:
`prepare_owned_rows` → `SyncSnapshot` → `apply_snapshot` → `model::list`.

```
date: 2026-11-11T00:24:44+01:00
command: cargo test -j 6 --manifest-path src-tauri/Cargo.toml lost_row
running 1 test
test owner_keys::tests::lost_row::a_file_row_with_an_unreadable_name_survives_the_key_walk ... FAILED
thread '...' panicked at src/owner_keys/tests/lost_row.rs:66:5:
the file row must survive the key walk, not vanish: []
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2671 filtered out
```

The empty listing is the file row gone: the folder is the listing's parent, so
only the file could have appeared.

## The fix (proven green, not committed)

`prepare_owned_rows` keeps the row when the canonical fetch is missing AND the
row is neither sparse nor a foreign row needing an envelope (`!sparse &&
!needs_envelope` — exactly the "name present but unreadable" case that took the
fetch). The match arm returns the input row instead of `continue`, so the
normal flow runs: `record_key` cannot resolve the name, `readable` is false, the
existing `blocked_fingerprint` is set, the record is stored and the prepared row
is pushed with `readable: false`. `stored_name` then returns the existing
`blocked_name` label ("Unavailable file <hex>") and `apply_metadata_file_row`
composes a path from it. No new state, no new key, no new wrap, no new column,
no new command, no new wire field.

The exact change at `src-tauri/src/owner_keys.rs:729` (old line):

```rust
                    // The listing/event raced deletion: omit the obsolete row. An
                    // unreadable name is not proof of deletion, so keep it blocked.
                    Err(error) if missing(&error) && !sparse && !needs_envelope => row,
                    Err(error) if missing(&error) => continue,
```

plus the two visibility words the test needs: `engine_bridge.rs` `mod syncflow;`
→ `pub(crate) mod syncflow;` and `syncflow.rs` `pub(super) fn apply_snapshot` →
`pub(crate) fn apply_snapshot`.

Green with that change:

```
date: 2026-11-11T00:30:17+01:00
cargo test -j 6 --manifest-path src-tauri/Cargo.toml lost_row
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2671 filtered out

cargo test -j 6 --manifest-path src-tauri/Cargo.toml owner_keys::tests
test result: ok. 76 passed; 0 failed; 0 ignored; 0 measured; 2596 filtered out
(75 before this slice; moved by exactly the 1 test added)

cargo test -j 6 --manifest-path src-tauri/Cargo.toml drive_v2::
test result: ok. 38 passed; 0 failed; 0 ignored; 0 measured; 2634 filtered out
(38 at the merge base 03c0d06c, unchanged)

cargo check --all-targets -j 6 --manifest-path src-tauri/Cargo.toml
Finished `dev` profile ... (0 errors)
```

## Why it is not committed: the pre-commit gate deadlocks on this file

The repo's pre-commit hook (`scripts/coord/hooks/run-pre-commit.sh`) runs
`scripts/check-file-sizes.sh --staged` and `scripts/coord/hooks/rustfmt-staged.sh`
on every staged `.rs` file. Both refuse this change, and the two rules cannot
both be met on `src-tauri/src/owner_keys.rs` as it stands:

- `src-tauri/src/owner_keys.rs` is exactly **1,200 lines** (the hard limit; it is
  not in `file-size-baseline.txt`). `rustfmt` 1.99.0 with the repo's
  `rustfmt.toml` (`max_width = 120`) reformats it to **1,217 lines** — over the
  limit. HEAD's own version already reformats to **1,216 lines**, so this is
  pre-existing, not caused by the fix.
- `src-tauri/src/engine_bridge.rs` is also not rustfmt-clean: a mechanical
  reformat is **564 changed lines** (775 → 520), while this slice allows only
  the `mod syncflow;` line there.

So `git commit` is refused with:

```
file-size ratchet: ok (staged): 5 source file(s) checked, 0 over the limit ...
rustfmt: 4 staged .rs file(s) are not rustfmt-clean:
  src-tauri/src/engine_bridge.rs
  src-tauri/src/owner_keys.rs
  src-tauri/src/owner_keys/tests.rs
  src-tauri/src/owner_keys/tests/lost_row.rs
run-pre-commit: 'rustfmt' refused the commit (exit 1)
```

The fix itself is correct and proven; the blocker is the repo's formatting/size
state, which a night slice may not rewrite (the brief allows only the two
visibility words in `engine_bridge.rs` and the fix in `owner_keys.rs`).

## What a human must do

Split `src-tauri/src/owner_keys.rs` (or raise the 1,200-line limit / pin the
toolchain to the rustfmt the repo was formatted with) and run `cargo fmt` on the
crate, then apply the change above. The test and the red/green evidence are in
`/home/guus/code/beebeeb.io/.claude/tasks/verification-evidence/desktop-v2/file-loss/`.

Windows proof: `nightctl native 65-the-file-row-survives-the-key-walk "lost_row"`
recorded; the runner runs it on Windows after review.
