# Windows session teardown and upload staging (task 1639)

Lock/sign-out hold `SESSION_TRANSITION`, revoke command and authentication
leases, and drain credential owners before stopping native providers and clearing
runtime keys. Browser attempts acquire a generation before opening the handoff;
`apply_session` validates it under the transition mutex before credential-store
writes or engine starts. Password login, 2FA and unlock attempts also capture
admission before waiting on that mutex. Startup restoration remains forbidden
after any earlier teardown. Failed drain keeps admission revoked for retry.

The `staged_payloads` SQLite journal records each upload copy before copying
plaintext. Finder create/modify transfer ownership to the durable operation
queue; Keep Mine retains a scope guard across every await. Dropping an incomplete
upload restores Conflict (or Error for ordinary uploads). The guard removes an
unchanged duplicate; if the original is absent or different, it preserves the
staged version and its journal for recovery. Server completion marks a payload
safe to remove before post-upload thumbnail work. Unlink failure never erases
its journal.

Windows sign-out inventories file caches, staging journals and legacy upload
resume paths before deleting native files. Queued/conflicted/failed changes
still refuse sign-out. A legacy payload whose redundant/server copy cannot be
proven requires recovery to a safe folder; sign-out never silently deletes it.
Native or external-file deletion failure retains references for retry. Journal
rows are forgotten only after successful removal. Old resume paths migrate into
the journal before queue/resume purges can forget them.

Audit scope: browser handoff, password login, 2FA, recovery/keychain unlock,
startup restore; Finder create/modify, Keep Mine, upload completion/thumbnail
awaits, operation/resume purges. Known-folder backup copies remain user files in
the sync root and are covered by native unsynced-file preflight. Hydration writes
to its tracked destination; it does not create finder-writes staging payloads.

Verification is in the workspace task 1639 `round5/` evidence: baseline red,
mutation checks, native CRLF Cargo gates and fresh-HEAD Linux/frontend gates.
These tests do not replace the separate Explorer/account-switch runtime matrix.
