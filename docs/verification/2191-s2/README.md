# 2191 S2 — Windows unknown-content baseline check

Owner: `l4-desktop`; desktop worktree `fs-l4-desktop-2191-s2-1`, branch
`fs/l4-desktop-2191-s2-1`, base `59c262e1`. Sync-engine ownership remains with L4
until the lead merges this PR; conflict-2190 / task 2204 waits. Desktop parity
starts after that merge. No installed application, real sync root or production
service is part of this validation.

Contract: workspace spec/plan `2026-10-25-2191-windows-unknown-baseline*`, S2
Tasks 6–10, amended by the approved `2026-10-26-2191-s2-throughput*` (workspace
PR 138). S3 partial completion and orphan resolution remain out of scope.

## Implementation boundaries

- The runner owns one job per engine/account. Four-file comparison quanta drain
  continuously; the local recorder has an independent task and 1 GiB quantum.
  A single file larger than that quantum can be recorded without starving.
- Current content is compared first. History is fetched only when needed; v2
  size filtering, pins and the ten-candidate cap apply. Old servers and shared
  history 404s use current content. No plaintext or plaintext hash is sent.
- The check calls the existing CF whole/range fetch decryptor, through a bounded
  single-attempt HTTP adapter. It deliberately bypasses hydration's internal
  retries: every request, including shared-key discovery, belongs to the ledger.
- `sync_state.unknown_check_ledger` stores the rolling request window, ciphertext
  reservations and global 429 cooldown atomically. Unused bytes are settled only
  after a live request/candidate; a restart cannot refund an unfinished request.
  Hash-mode oversized candidates reserve their whole upper bound on an untouched
  day. Legacy JSON envelopes use a conservative base64 wire bound.
- A resume digest includes the compared prefix's chunk lengths and chained hash.
  V1 metadata has no reliable chunk stride; guessing size/count would skip the
  wrong bytes. Every saved prefix is reread and verified before skipping requests.
- Match promotion is conditional on the row/path still being eligible. Chunk
  mode rehashes under an exclusive handle. Hash mode preserves later local edits.
  Divergence keeps a new `(this PC)` copy before same-handle deletion, with a
  durable watcher guard. Entry-driven recovery also handles a missing original
  or a deleted/trashed database row. S2 kept entries have `content_known = false`
  and use their own recovery path, never the older prune retry.

## Validation and release boundary

Complete command output is retained on the Mac under
`~/code/bb-gate-logs/2191/l4-s2-implementation/`. Red runs assert nonzero failed
counts; the final green and mutation summary accompany the desktop PR. Rust
builds/tests run on the testrig; the Legion gate uses its own
`C:\bb-win\gate\desktop-l4-2191-s2` clone and checks the exact pushed SHA.

The test fixtures use temporary files/databases and loopback HTTP only. A
20,000-file scheduling test is deterministic arithmetic over the actual quantum
and yield decisions, not a throughput measurement. The encrypted-transport
fixture proves current-first behavior and byte accounting across real quanta.

The lead owns the independent sync-safety review and merge. The code-review
skill's independent agent lanes are unavailable under this program's explicit
no-child-agent rule; author inspection is not independent approval. Real CFAPI
upgrade, Explorer overlays and Free up space checks remain the release/device
rungs owned by Guus and the lead. Linux compilation is not native Windows proof.

Metered/unknown connections retain the deliberately slower 2 GiB/day allowance.
An edited or missing kept copy never authorizes deleting the original; it stays
protected. A crash during an in-place copy refresh can likewise leave the
original protected for inspection. Downgrading to a pre-guard client restores
that client's original hazard. No version bump or release is part of this PR.
