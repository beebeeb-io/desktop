# Windows callback and sign-out boundary (task 1639)

Implements the connection-key ownership planned in task 1281, on top of task
1538's confirmed runner stop. The seven unmerged 1281 commits do not contain
this teardown; they are not cherry-picked. macOS File Provider registration,
IPC and cleanup paths are unchanged.

Each Cloud Files connection has a monotonically increasing context ID and a
revocable `CallbackGate<EngineBridge>`. Fetch, notify and thumbnail callbacks
hold leases. A thumbnail's Initialize/GetThumbnail pair retains the generation
ID, not an engine/key. Revocation stops admission, cancels pending network
futures, drains leases through the final OS transfer, and disconnects the saved
connection key. A failed disconnect retains its key for retry. Runner stop
revokes both before and after stopping, covering startup racing teardown.

The runner tracks a weak lifetime marker that is dropped *after* ApiClient
zeroizes its key and token. Windows lock/sign-out wait for this marker, covering
watcher scans and detached heartbeat work too. Forced runner shutdown also
aborts its heartbeat. Timeout is a failure, never a successful lock. Login,
unlock and teardown are serialized. New-account login requires explicit
successful sign-out; a new runner waits for the previous credential owners.

Windows sign-out refuses pending operations (including paused/exhausted ones),
conflict/error rows, untracked files, busy handles, unavailable roots and dirty
placeholders. The account remains signed in with its local DB/staging intact;
the user must unlock, finish syncing/resolve errors, or move untracked files out
before retrying. There is no implicit discard consent. No forced-discard UI or
new on-disk quarantine format is introduced.

For clean files, cleanup opens exclusive Win32 handles without reading content,
checks native placeholder identity, `InSyncState`, and `ModifiedDataSize`, then
deletes by the same handles. A failed dehydration is never converted into file
deletion. Directories are removed only when empty; no recursive deletion is
used. Cache failures retain DB references for retry. Only successful cleanup
clears account rows/cursors/activity/resume state. Failures propagate to the
existing command error surface. Root and shell unregistration follow; server
logout is bounded and best effort when offline.

COM factories remain in their original apartments without credentials. Sign-out
removes shell bindings, and re-login reinstates them without reusing a revoked
class cookie. All account access still requires a current lease.

Windows CI must execute the portable lease/DB tests and the existing platform
suite. Native acceptance remains required: held chunk response across lock,
post-lock read/error/counter checks, dirty/busy/untracked preservation, failed
purge retry, and A/B account switching twice in one process. See workspace
`.claude/tasks/verification-evidence/1639/NATIVE.md` for commands. Root relocation
and single-instance work remain under 1281/the later epic lifecycle gate; this
change does not claim those scenarios have passed.
