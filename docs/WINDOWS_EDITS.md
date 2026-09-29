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
to serialized task 1641; immutable download versions and ambiguous completion
recovery remain the subsequent epic work.
