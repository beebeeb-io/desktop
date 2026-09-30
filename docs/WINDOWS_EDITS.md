# Windows tracked edits (task 1640)

Design authority: task 1640 / epic 1638, recorded 2026-09-29 before implementation.
This is the Windows watcher contract; macOS File Provider IPC remains unchanged.

- A close is a hint, not proof of a user write. Deleted closes and provider-process
  closes are ignored; reads and hydration are distinguished from edits by a
  plaintext SHA-256 comparison against the confirmed `local_hash` in StateDb.
  An unavailable legacy baseline is treated conservatively as an edit.
- Both close debounce and periodic scanning examine tracked resident files.
  Replacement at a tracked path retains that path's server ID. Namespace events
  settle for 400ms so rename-old/rename-new and delete/recreate save sequences
  can be recognized before sending trash/move operations.
- Each distinct settled content snapshot is copied to `.beebeeb/windows-writes`,
  flushed to disk, and queued with the stable ID and a numeric base-version
  precondition before its row becomes Uploading. Duplicate notifications compare
  against the latest queued hash. Successive edits retain separate payloads;
  later base versions wait for their predecessors. Queue contents survive restart.
- Windows edit transport retries remain eligible beyond the ordinary 25-attempt
  horizon. Auth/quota/permission recovery still follows the shared queue policy.
  A server 409 retains both the staged payload and a uniquely named visible
  conflict copy, marks Conflict, and stops retries of that stale version. The
  server winner is never overwritten without resolving its precondition.
- Upload/hydration success records the server-confirmed hash. It never records a
  scan as a baseline. The periodic in-sync pass verifies resident bytes against
  that hash with no pending upload. Unknown hashes and errors fail closed.
- Native last-write tracking clears IN_SYNC synchronously on a user write,
  including the interval before debounce/queue insertion. The shell dehydration
  callback additionally checks the durable queue and baseline. Application
  reclamation compares bytes while holding a referenced exclusive CF oplock
  through dehydration. Dirty/not-cloud errors are failures, not successful
  zero-byte evictions that would incorrectly mark the row cloud-only.

The staging and confirmed-hash fields reuse the current schema and upload API.
No crypto, schema, dependency, public API or production change is required.

CFAPI references: [last-write in-sync policy](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_insync_policy),
[exclusive oplocks](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfopenfilewithoplock),
[protected-handle references](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfreferenceprotectedhandle),
[dehydration notifications](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type).

Native Windows callback ordering, Explorer/Notepad/Office behavior and clean
reclamation must be gated on the real PC. Portable tests and adapter compilation
are supporting evidence, not native passes. Full-file hashing is bounded in
memory (64 KiB buffer) but costs I/O proportional to resident file size; the epic's
large-tree performance rung remains necessary. Remote subtree deletion belongs
to serialized task 1641; ambiguous completion recovery remains subsequent epic
work. Partial reconstruction uses immutable object versions when the base is
no longer current; unavailable historical bases retain the pending snapshot.

## Round 3 correction (2026-09-30, Guus task 1640 ruling)

- Hydration carries its downloaded version alongside plaintext; only that pair
  may become the confirmed baseline. Current DB metadata can advance independently.
  Mutable chunk downloads must validate the same server identity before/after.
- Missing ranges do not imply clean content. Native modified data and in-sync
  state distinguish clean cloud placeholders from dirty partial files. Preserve
  dirty ranges plus EOF in a durable snapshot/queue before network work; fill
  missing bytes from the captured base version, never the newer remote winner.
- Each conflict choice captures its operation chain and preserves its staged
  saves as recoverable copies before retiring those operations after success.
  Saves arriving during resolution stay queued, with their base reconciled to
  the resolved version. Failures keep operations and payloads recoverable.
- `.beebeeb/windows-writes` is reserved engine storage. Sign-out holds native
  directory handles and only deletes these two directories after proving their
  contents empty. Pending operations, orphan payloads, unknown children and
  reparse points must refuse sign-out without deleting payloads.

Round 3 native unit fixtures use Windows file handles, range writes, append,
SQLite restart and real local HTTP, with injected CFAPI modified-range metadata.
They do not register sync roots. Real CFAPI callback behavior and complete
edit/upload/sign-out on a registered root remain the independently owned QA rung.

Legacy dirty partials without a recorded base still enqueue their packed ranges
and EOF durably. Reconstruction reports an unknown-base error and retains the
snapshot; it must never invent a version precondition for those bytes.
