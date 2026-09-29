# Native parity fixtures (1612)

Design recorded before implementation, 2026-09-29. This is a reusable **test API**,
not a production server, login service or evidence of native shell execution.
The fixture implements the desktop HTTP subset against isolated in-memory state.
Full-stack server semantics, CFAPI, FUSE and File Provider remain separate gates.

The checked-in manifest is the oracle: two accounts, three folder levels,
repeated basenames, Unicode, empty content, base/local/remote conflict versions,
a remote-trash subtree, a three-chunk interrupted upload, an image and a video
with a precomputed poster, expired session and revoked recipient access.
IDs, names, bytes, SHA-256 and versions are stable; synthetic keys, tokens and
encryption nonces are generated per setup and never serialized or logged.
Video poster retrieval does not certify video decoding or poster generation.

The API binds IPv4 loopback on a random port only. Native Rust harnesses own the
fixture in-process, obtain credentials in memory and pass them directly to the
test bridge. No credentials in argv, environment, manifest or desktop.toml.
The same request dispatcher is tested without sockets on restricted sandboxes;
that rung cannot substitute for the HTTP/bridge rung.

Setup verifies exact positive object/scenario/account counts and every plaintext
hash. Controls advance remote versions, trash a subtree, expire a session,
revoke a share and fail one upload chunk before acknowledgement. Requests and
effects have separate nonsecret counters. Unsupported routes fail explicitly.
Cleanup stops and joins the owned listener, clears objects/uploads/accounts,
removes the owned temporary directory and asserts zero residual state. Cleanup
never deletes arbitrary caller paths. Run setup/exercise/teardown twice.

Verification: Bun manifest assertions first fail on origin/main (missing
corpus); Rust fixture/router assertions and bridge integration then run twice.
Deliberate wrong hash and altered state/fault/authorization outcomes must fail
the named assertions. Preserve command exits, executed counts and limitations.

## Files and consumption

- `fixtures/native-parity/manifest.json`: public oracle (no secrets), 14 objects,
  9 scenario IDs, 16 verified blob entries. Folder IDs end in 1–3 and 8;
  file IDs end in 4–7 and 9–14. Account A owns 13 objects, account B owns 1.
- `fixtures/native-parity/plaintext/`: synthetic text, zero-byte file, 20-byte
  three-chunk upload, generated 32×32 PNGs and a one-second 32×32 H.264 MP4.
  All assets were generated locally for this test; no stock/personal media.
- `src-tauri/tests/support/native_parity.rs`: `Corpus`, `Fixture` and the local
  API. No runtime dependency was added. `lib.rs` includes the bridge checks
  only with `cfg(test)`; there is no fixture code in a shipping build.
- `src-tauri/tests/support/bridge.rs`: working example using the private
  production `ApiClient`, `EngineBridge`, sync snapshot/delta and StateDb.

Run from the desktop checkout (keep the corpus beside `src-tauri`):

```sh
bun test tests/nativeParityCorpus.test.ts
cd src-tauri
cargo test --locked --lib native_parity_tests::corpus -- --nocapture
cargo test --locked --lib native_parity_tests::http_ -- --nocapture
```

The first Rust filter executes 9 socket-free tests; the second executes 3
loopback integration tests. A socket bind failure is a failed/unavailable rung,
never an ignored or successful case. Run both filters twice. Each cycle owns a
new temporary directory, listener and in-memory account pair. `cleanup()`
requires 14 removed objects, 2 removed accounts, and zero objects, accounts,
upload sessions, owned paths and listeners remaining. The HTTP tree test also
runs its entire cycle twice internally. DB/bridge handles must drop before
cleanup on Windows. RAII joins the listener and removes its TempDir on failure.

For 1615's Windows test module, include the support module using `#[path]`, call
`Fixture::setup()` and `start_http()`, then `credentials("alice")` or `("bob")`.
Pass these memory values to `ApiClient::new`/the native test runtime; never print
or persist them. Use a fresh native provider process/root per scenario plus the
separately required same-process account-switch case. Parent/child harnesses
must carry credentials over their private IPC, not command-line arguments.
Do not add a production login bypass for this fixture. The fixture deliberately
has no OPAQUE/browser-login/UI endpoints. A packaged GUI login smoke uses the
real isolated server/account setup from audit F instead.

Linux FUSE tests and macOS File Provider regression tests use the identical
module, manifest and memory credentials. Only their adapter/root provisioning
changes. A manifest hash verifies bytes; it does not prove a native callback ran.
Capture adapter callback counts, native paths/registrations and screenshot
proof in the owning native task. The portable bridge test intentionally uses
`hydrate_file` to an owned synthetic TempDir, not the Windows CF transfer path.

## Scenario controls and API contract

| Scenario | Initial state and trigger | Observable assertion |
| --- | --- | --- |
| nested-tree | three folders, repeated `readme.txt`, Unicode, empty file | Alice snapshot has 12 visible objects; exact parent IDs, decrypted paths and hashes |
| edit-conflict | `conflict.txt` version 1; stage manifest's local bytes, call `remote_edit()` | version 2 remote bytes; stale base 1 `/uploads/init` returns 409; local and remote hashes differ |
| subtree-trash | folder 8 with child 9; call `trash_subtree()` | one `file_trash` op, both objects absent, Alice snapshot count 10 |
| interrupted-upload | object 10 starts `is_uploading`, excluded from snapshot; init declares its ID and 20 bytes | 3×8/8/4-byte chunks, chunk 1 fails once with 503 before acceptance; retain chunk 0, retry, exactly 3 accepted chunks and 1 commit |
| image-thumbnail | object 11, medium/large PNG | authenticated bytes decode to 32×32; no original chunk requests |
| video-poster | object 12, precomputed medium/large PNG poster | same thumbnail assertions; MP4 plaintext hash independently available |
| expired-session | `expire("alice")` | Alice 401, Bob still 200 |
| revoked-share | owner Alice, recipient Bob, direct read-only X25519-wrapped file key for object 13; `revoke_share()` | recipient initially decrypts; then 403/empty incoming shares and removed shared mirror; Alice ownership unaffected |
| two-accounts | separate random keys/tokens | Alice cannot read object 14; Bob cannot read object 4; owned snapshots remain separate |

Supported HTTP routes: `/api/v1/sync/{snapshot,ops?since=N}`,
`GET /api/v1/files/:id`, `/chunks/:index`, `/thumbnail/:variant`,
`GET /api/v1/shares/invites/incoming`, `POST /api/v1/uploads/init`,
`PUT /api/v1/uploads/fixture-upload/chunks/:index`, and
`POST /api/v1/uploads/fixture-upload/complete`. All require the corresponding
in-memory bearer. Unsupported routes return 404. Small thumbnails intentionally
return 404; medium and large are present. Content uses the pinned core's real
AES-GCM envelope and textual UUID KDF input. Names are encrypted too. Timestamps
are server-shaped RFC3339 values, not a fabricated integer shortcut.

The upload endpoint is a bounded fixture for object 10, not a general storage
service; stale-base conflict requests for object 7 are also accepted as negative
cases. HTTP 503 is a deterministic interruption before acknowledgement, not an
actual lost TCP acknowledgement, crash or durable server restart. The HTTP
upload test uses the production API client but does not certify the durable
upload queue's retry policy. Native/full-stack task owners extend these cases
without interpreting fixture idempotency as real server idempotency.

`Counts` separates requests, original chunks, thumbnails, injected faults,
initializations, accepted chunks and committed versions. Reports contain no
request headers/bodies or credentials. The fixture only binds `127.0.0.1:0`,
limits headers/body/timeouts, and joins its single worker on teardown. It never
contacts a production API or registers an OS sync root.
