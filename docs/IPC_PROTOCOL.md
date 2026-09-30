# Daemon IPC protocol (Unix socket)

Task 1670 issue 3. This is the contract between the desktop daemon
(`src-tauri/src/ipc_socket.rs`) and its socket clients: the macOS File Provider
extension (`BeebeebFileProvider/XPCBridge.swift`) and the Rust readiness probe
(`wait_for_file_provider_ipc_ready`). The Linux FUSE prototype (`src-tauri/src/linux_fuse`)
does not use this socket. Unix sockets do not exist on Windows, where Cloud Files runs
in-process.

Implementations that must stay in step:

| Half | File |
| --- | --- |
| Rust framing | `src-tauri/src/ipc_frame.rs` |
| Rust server | `src-tauri/src/ipc_socket.rs` (`handle_connection`, `hydrate_over_ipc`) |
| Swift framing + exchange | `BeebeebFileProvider/IPCFraming.swift` |
| Swift caller | `BeebeebFileProvider/XPCBridge.swift` |
| Write-queue idempotency (Rust) | `src-tauri/src/ipc_write_dedup.rs` (table), `dedup_write` in `ipc_socket.rs` |
| Write-queue idempotency (Swift) | `IPCWriteKey` / `IPCWriteRequest` in `BeebeebFileProvider/IPCFraming.swift` |
| Readiness probe (Rust client) | `wait_for_file_provider_ipc_ready` in `src-tauri/src/lib.rs` |

## Framing

A **frame** is one compact JSON value followed by a single `\n` (0x0A).

- Compact JSON escapes every control character inside a string (`\n`,
  `\u000a`), so a raw 0x0A byte can only be the delimiter. Both encoders assert
  this (`serde_json::to_vec` on the Rust side; `IPCFraming.encodeRequest`
  refuses to send a request containing one).
- One request per frame, client to server. Exactly one final reply frame per
  request, server to client. The connection stays open for further requests.
- Empty and whitespace-only lines are ignored.
- Requests are limited to 1 MiB (`MAX_REQUEST_BYTES`). A longer message gets an
  `Error` frame and the connection is closed (the stream cannot be
  resynchronised). Replies are limited to 32 MiB on the Swift side
  (`IPCFraming.maxReplyBytes`); over that the client reports an honest
  "reply larger than Finder can accept" error.

```mermaid
sequenceDiagram
    participant X as Finder extension (Swift)
    participant D as Daemon (Rust)
    X->>D: {"HydrateFile":{...,"progress":true}}\n
    D-->>X: {"HydrateProgress":{"done":0,"total":T}}\n
    D-->>X: {"HydrateProgress":{"done":N,"total":T}}\n
    D-->>X: {"Ok":{}}\n
    Note over X,D: any other request: exactly one reply frame
```

### Why (what was wrong before)

- The daemon treated one `read()` of at most 64 KiB as one whole request and
  wrote replies with no delimiter. On a `SOCK_STREAM` socket a request can arrive
  in several reads and a client cannot know where a reply ends.
- The Swift client did one unlooped `write()` and one `read()` of at most 65536
  bytes, then parsed. A reply over 64 KiB (a folder of a few hundred items) was
  always truncated.
- The daemon answered a successful hydrate with the bare JSON string `"Ok"`
  (a unit enum variant). `JSONSerialization` rejects a top-level string unless
  fragments are allowed, so **every successful hydrate surfaced as "daemon
  response was not valid JSON"** after the download had already finished. The
  success reply is now the object `{"Ok":{}}`.

## Hydrate progress

`HydrateFile` accepts an optional `"progress": true` (default false). When set,
the daemon sends zero or more `HydrateProgress` frames before the final reply:

```json
{"HydrateProgress":{"done":10485760,"total":31457280}}
```

`done` and `total` are plaintext bytes; `total` is 0 when the server did not
report a size. The first frame is `done: 0` as soon as the file's metadata and
key are known; then one per decrypted chunk. The final reply is `{"Ok":{}}` or
`{"Error":{"message":"..."}}`.

Clients that do not opt in (the 0.8.6 extension) receive no progress frames,
because they read exactly one reply and would mistake the first progress frame
for it.

### Cancellation

While a hydrate runs the daemon also watches the socket. If the client hangs up
(Finder cancelled the `Progress`; the Swift side calls `shutdown()` on the
socket) the daemon drops the download, writes nothing to disk, and restores the
file's status (`DownloadingStatusGuard` in `engine_bridge.rs`).

## Timeouts (Swift client)

Applied as `SO_RCVTIMEO`/`SO_SNDTIMEO`, i.e. per `read()`/`write()` call:

| Call | Timeout | Reason |
| --- | --- | --- |
| list / item / queue delete / queue create+modify without contents | 30 s | Database work in the daemon; answers in milliseconds. |
| queue create / modify WITH contents | 600 s | The daemon copies the whole file into staging (`StagedPayload::copy`, synchronous) before replying and sends nothing meanwhile. A timeout here is a duplicate hazard: the extension reports failure, the daemon still queues the upload, Finder retries and queues it again under a fresh id. 600 s covers 15 GB at 25 MB/s (a same-volume APFS copy is a clone, near-instant). A copy longer than that still times out; task 1684 closes that with a stable `request_id` the daemon dedups on (next section). |
| hydrate | 600 s idle | Each progress frame restarts it. The daemon's HTTP client allows 30 s per request (`api_client.rs`) and a hydrate makes one metadata request plus one per chunk; an older daemon sends no progress and is silent for the whole download. 20x the per-request ceiling also covers `download_kbps_limit` pacing sleeps. |

A timeout surfaces as `BeebeebIPCError.timedOut(seconds:)`
("Beebeeb's sync engine did not answer within N seconds..."), never as a JSON
error. No error message ever contains raw reply bytes.

## Write-queue idempotency (`request_id`, task 1684)

`QueueFinderCreate` and `QueueFinderModify` accept an optional `"request_id"` string.

**Why it has to be derived, not random.** When the extension's socket call times out, the
system retries `createItem` / `modifyItem`. That retry is a NEW invocation, so it cannot know a
per-call UUID the first attempt generated. A random id per call would be different every time
and deduplicate nothing. The key is therefore computed from the stable inputs of the logical
operation, so the retry recomputes the same value.

```mermaid
sequenceDiagram
    participant S as System (fileproviderd)
    participant X as Extension (Swift)
    participant D as Daemon (Rust)
    S->>X: createItem(template, contents)
    X->>D: QueueFinderCreate{request_id: K}
    Note over D: InFlight(K): copying a huge file...
    X--xD: socket timeout (600 s), extension reports failure
    S->>X: createItem(same template, contents)  (retry)
    X->>D: QueueFinderCreate{request_id: K}  (same K)
    Note over D: K is InFlight: wait, do NOT enqueue again
    D-->>X: WriteQueued{item} (the first result)
    X-->>S: success
```

### The key

`request_id` = lowercase hex SHA-256 (64 characters) of this canonical byte string. Every
field is `s<utf8 byte count>:<bytes>;` for a string, `n;` for an absent string, `i<decimal>;`
for an integer; the fields are concatenated in this order (length prefixes make `("ab","c")`
and `("a","bc")` different):

| Operation | Fields, in order |
| --- | --- |
| create | `"bb-write-v1"`, `"create"`, parent item identifier, filename, kind (`file`/`folder`), content type, content size in bytes, content mtime in nanoseconds |
| modify | `"bb-write-v1"`, `"modify"`, item identifier, base version identifier, parent item identifier, filename, kind, content type, changed-fields bitmask (`NSFileProviderItemFields.rawValue`), content size, content mtime in nanoseconds |

"Content size / mtime" come from `stat()` on the file the system staged for the write.

- **Only requests that carry contents get a key.** A folder create, a rename or a move is
  answered from the database in milliseconds under the 30 s timeout; there is nothing to
  deduplicate and the request is sent without one. A request whose staged file cannot be
  `stat()`ed is also sent without one (never a made-up one).
- **The staged file's path is not an input** (it may differ between attempts) and neither is its
  inode: if the system re-stages a retry into a new file, an inode in the key would make every
  retry a different key. Size + nanosecond mtime survive a metadata-preserving clone/copy.
  Whether the system really hands the retry the same size and mtime is **not verified on a
  device** (see "Device rung" below); the extension logs the first 12 characters of each key so
  that rung has evidence.
- The Swift half is pinned by a test vector computed independently in Python
  (`BeebeebFileProviderTests/main.swift`).

### The daemon table

`WriteDedup` (`ipc_write_dedup.rs`) is an in-memory map `request_id -> InFlight | Done`,
shared by all connections, bounded to 4096 entries and a 30 minute TTL.

| A request with key K arrives while... | The daemon... |
| --- | --- |
| K unknown | runs the work (on the blocking pool, detached from the connection, so the copy finishes and is recorded even though the client hung up) |
| K `InFlight` | waits for the first run and returns **its** reply; never enqueues a second upload |
| K `Done` within the TTL and the result is still true | returns the stored reply without enqueueing |
| K `Done`, but the row it created is gone or renamed | treats it as a NEW operation (see below) and runs it |
| K `Done` past the TTL | forgets it and runs the work |
| K was used for a different (operation, parent or item, filename, kind) | does not merge them; runs the work and leaves the original entry alone |
| the first run panicked | the waiters are released and one of them runs its own work |
| the table is full of in-flight work | runs the work without deduplication rather than refuse a user write |
| no `request_id` (0.8.6 extension) | exactly the pre-1684 path: the work runs inline, nothing is remembered |
| `request_id` empty, over 128 bytes or containing a control character | `Error{"request_id must be 1 to 128 printable bytes"}`; nothing is queued |

Only `WriteQueued` replies are remembered. An `Error` is delivered to requests that were
already waiting on that run, but a later retry runs the work again (a failed attempt must not
poison the key).

### False-dedup risk (two genuinely different operations, one key) and how it is bounded

1. **Different file, same key:** needs parent, name, size and nanosecond mtime all equal, which
   is the same file going into the same folder (a name collision regardless). Plus the daemon
   refuses to merge across a different operation/parent-or-item/filename/kind.
2. **The same inputs recurring as a genuinely new operation inside the TTL:** create `a.txt`,
   delete it (here or on another device), copy the identical file back with its mtime preserved.
   Without a guard the second create would be answered with the first, and the file would
   silently never upload. The daemon therefore validates a stored create/modify reply before
   returning it: the row it reports must still exist **and still have the same filename**;
   otherwise the entry is dropped and the request runs. `a_cached_create_is_not_returned_once_its_row_is_gone`
   pins this.
3. **Accepted residual risk:** if the user renames the created item and a timed-out create of
   the same name is retried afterwards inside the TTL, the validation sees a renamed row, calls
   the stored reply stale and runs the retry as a new create (one duplicate, the pre-1684
   behaviour). That needs a retry to arrive after the user has already acted on the result, which
   the File Provider retry loop does not do in practice; it is unmeasured.
4. **Daemon restart forgets the table.** A retry that arrives after the daemon restarted is
   treated as new and can queue a second upload. Persisting keys would need a schema change;
   the window is the copy time plus a restart, and the pre-1684 behaviour was a duplicate every
   time.

### Device rung (not proven on this branch)

CI proves the framing, the key derivation, the request shape and the daemon table. It does not
prove that the real File Provider system retries with the same staged size and mtime, or that
the retry happens within 30 minutes. The next macOS alpha must force a write-queue timeout on a
large file (or lower `stagedCopyTimeoutSeconds` for a test build) and check the unified log for
the same `request_id=<12 hex>` on both attempts and exactly one queued upload.

## Version skew (app update in progress)

| Extension | Daemon | Behaviour |
| --- | --- | --- |
| new | old | The old daemon replies with no delimiter and keeps the connection open, and answers a hydrate with the bare string `"Ok"`. `IPCFrameReader` accepts a buffer that is already one complete JSON value without waiting for a newline, and `IPCFraming.decodeReply` maps a bare `"Ok"` to `{"Ok":{}}`. The old daemon ignores the unknown `progress` field, so no progress is shown. |
| new | old (before 1684) | The old daemon's `serde` ignores the unknown `request_id` field, so the request works and is simply not deduplicated (the pre-1684 behaviour). |
| old | new | The old extension writes its request with no delimiter and does not close: `FrameReader` accepts a buffer that is already one complete JSON value. It requests no progress, so it gets one reply, now `{"Ok":{}}\n` (which its parser accepts). It sends no `request_id`, so its write-queue requests take the pre-1684 path. |

## Tests

| What | Where | Truth line |
| --- | --- | --- |
| Rust framing units | `ipc_frame.rs` `mod tests` | `cargo test` counts |
| Rust socket end-to-end (>64 KiB reply, split request, several requests per connection, unframed legacy request, oversize, bad line, `Ok` shape) | `src/ipc_socket_framing_tests.rs` | `cargo test` counts |
| Hydrate progress + cancellation | `engine_bridge.rs` tests `ipc_hydrate_*` | `cargo test` counts |
| Swift framing + exchange over a real `socketpair` | `BeebeebFileProviderTests/main.swift` via `scripts/test-ipc-framing.sh` (macOS CI job "File Provider Swift (macOS)") | `ipc-framing: N passed, 0 failed`, N asserted |
| Swift key derivation (same inputs same key, any input differs, pinned vectors), request shape (`request_id` on BOTH create and modify, absent without contents) and file fingerprint | same file (10 tests of the 35) | same |
| Daemon table: concurrent same key runs once, repeat after completion, different keys, TTL expiry, key reused for another request shape, stale result, failed result not remembered, panicking leader, capacity | `ipc_write_dedup_tests.rs` | `cargo test` counts |
| Daemon over the real socket, counting REAL queued operations in the state DB | `ipc_socket_framing_tests.rs` (`concurrent_creates_with_one_request_id_...`, `a_repeat_after_completion_...`, `different_request_ids_...`, `a_request_without_a_request_id_...`, `a_cached_create_is_not_returned_...`, `concurrent_modifies_...`, ...) | `cargo test` counts |
| XPCBridge call sites still use the builder and pass the key inputs (XPCBridge is not compiled into the Swift harness) | `scripts/check-ipc-timeouts.py` (+ `--self-test`, 14 mutations) | `ipc-timeout guard: 3/3 call sites correct` |
