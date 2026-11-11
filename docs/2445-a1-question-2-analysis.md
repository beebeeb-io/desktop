# 2445-A1 Question 2: What key did the engine use? Could it have written anything?

## Answer

**The engine on v2-next cannot start with a zero, stale, or wrong master key.**

Every engine start reads its 32 bytes from `acct.session.lock()` via
`keys_for_engine_start` (session_reauth.rs:624). If no `Session` exists in
memory, the function returns `EngineStartBlocked::NoSession` and no engine
spawns. The ONLY ways to place a `Session` in memory, each of which installs
a real, verified 32-byte key:

| Path | Where the key comes from | Verification |
|---|---|---|
| Recovery phrase unlock (`install_recovered_session`) | `recover_from_phrase` → HKDF from the BIP-39 words | `recovered_key_matches_account` (server confirms) |
| Device approval (`apply_session` via `device.rs:install`) | The approving device relays the key over ECDH | The approver holds the real key |
| Startup restore (`restore_stored_session` → `load_session_from_keychain`) | Windows Credential Manager | Was verified on the sign-in that stored it |
| Browser handoff (`apply_session`) | Handshake from the signed-in browser | The browser holds the real key |

A password-only `desktop_login` calls `store_first_sign_in`, which writes the
session TOKEN to the credential store but **never sets `acct.session`** (no
`master_key`). `vault_unlocked` reads `false`. `start_engine_if_possible` is
never called on this path. A startup restore after such a sign-in fails at
`vault.unlock()` ("secret not found") and returns `false` — no engine.

**Therefore:** the engine in the test build could only have started after a
successful recovery-phrase unlock or device approval, both of which deliver the
correct master key. The 44,398 empty names are NOT from a zero or stale key.

## What caused the empty names then?

The empty names come from the metadata ingest path: `apply_metadata_file_row`
(meta_2.rs:395) calls `owner_keys::stored_name`, which reads the
`owned_key_record` for the file id and decrypts its `name_encrypted` using
`record_key`. If that returns `None`, it falls back to
`resolve_relative_path` (fs.rs:219), which tries `decrypt_name` and then
the server-side `path`/`name` fields — which are empty for zero-knowledge
encrypted files. Result: `""`.

The `prepare_owned_rows` step runs before `apply_snapshot` and should
populate the records. If any record fails to resolve (e.g. the server's
canonical listing was unavailable, or the name blob couldn't authenticate),
that record is stored as blocked, and `stored_name` returns a placeholder
`[blocked]` name rather than `""`. An empty-string name means neither
`stored_name` nor `resolve_relative_path` could decrypt — which SHOULD NOT
happen with a correct master key for owner-class items.

**A2 owns `owner_keys.rs` and the listing path.** My fix scope is the engine
gate (no engine without a verified key) and the sign-in error handling; the
name-resolution path is A2's.

## Could the engine have UPLOADED or re-keyed anything?

**No, with high confidence.** The engine derives per-file keys from
`api.master_key()` using HKDF. With the correct key:

- An upload seals the file name and content under the derived file key →
  other devices can decrypt → **no data corruption**.
- `owned_key_records` are written with wraps derived from the same key →
  **consistent**.
- No path on v2-next constructs a zero-byte or placeholder master key.
  The `Session::master_key` field is `[u8; 32]` with no `Default` impl —
  it must be explicitly set.

The engine's upload paths (`upload_version`, `upload_session_body`,
`upload_thumbnails_for_plaintext_media`) all read `api.master_key()`,
which is the same correct key the snapshot ingest used. No custody
registration, adoption, or re-keying can produce a foreign key when the
master key is correct.

**Verdict: the test build's engine had the correct master key. No data was
corrupted.** The 44,398 empty names are a display/ingest bug in the name
resolution path, not a crypto failure.
