# Task 1672: local namespace and deletion

2026-09-30 — Narrow legacy repair beneath spec 50 §§3, 4.4 and 5.1.1.
StateDb remains the only transaction authority. No path map, lock, or deletion
journal is added. The existing operation queue owns namespace intent and unresolved
delete observations; files.path remains the namespace binding and Trashing the
materialization tombstone. These records are inputs to spec 50 P1/P2 migration.

- Publish a local rename and all tracked descendant paths atomically with its
  metadata operation. Keep acknowledged namespace operations until matching remote
  metadata arrives, preventing older snapshots from reverting local names.
- Atomically enqueue one TrashFile and tombstone its tracked subtree. Duplicate
  callbacks and children of a trashed folder cannot enqueue redundant deletes.
- Carry callback generation and placeholder identity into dispatch. A stale
  generation, a different path occupant, unknown identity or pending upload
  finalization cannot authorize deletion. Missing files alone never imply trash.
- Retain unresolved delete observations in the existing queue, with visible review
  errors. Scan retries these observations by their original identity; unknown
  identities remain review work, never adopt a new occupant by pathname. Pending
  observations block materialization at the affected binding.
- Wake the existing transfer loop after durable enqueue. No additional worker or
  lock owns deletion.

Verification: production watcher → StateDb → production HTTP worker tests first
fail on the baseline TrashFile count. Cover before/after PATCH, moves, subtree,
restart, duplicates, stale metadata, unknown/reused paths, remote echoes,
finalization and HTTP retry. Independently mutate binding, tombstone and dedup.
Linux locked Cargo, bun, tsc, lint. Native clean CRLF checkout build/test; registered
root tests and interactive app are PENDING while task 1654 owns PID 61548.
Native acceptance: Explorer Delete, Shift+Delete, Remove-Item and folder delete;
Trash in 30 seconds, no resurrection after scan/restart, web restore SHA-256.
Process replay tests do not claim spec 50 power-loss barriers.

Resume review, 2026-09-30: unresolved observations fence both their observed path
(subtree included) and supplied identity, even when that identity is unknown.
They never authorize a later occupant. Recheck the expected delete binding inside
the queue transaction. Coalesce superseded namespace operations, retarget descendant
namespace intent on folder moves, and skip removed operations in a loaded worker
batch. Callback dispatch must match both generation and the owning bridge.
A successful HTTP request followed by a crash before its DB acknowledgement can
still be retried by the existing at-least-once queue; this change does not promise
exactly-once network delivery across that crash boundary.
An authoritative file_restore event releases settled subtree tombstones only when
no pending operation owns the subtree or its ancestors, then requests fresh
metadata. Without this explicit release, the legacy Trashing marker can hide a
web-restored file indefinitely. Pending local trash continues to win over a stale
restore event. Newly observed remote descendants of a tombstoned folder must also
remain unmaterialized.
The existing transient engine-delete suppression is narrowed to (path, server ID)
with its existing TTL. It may suppress duplicate completion/deleted-close echoes
only while that identity has no DB binding; a restored or reused binding always
gets a fresh delete decision. This is echo suppression, not namespace authority.
