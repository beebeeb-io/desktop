// Snapshot fixtures for every popover state (task 1683 slice 3), shaped exactly like
// `popover_snapshot`'s JSON (src/popoverContract.ts `PopoverSnapshot`). Shared by the bun unit
// tests and the local Playwright rung, so both render the same data. The content is the design
// file's mock identity (`sam@example.eu`, the same files and sizes), so a render can be put
// beside the design's render. `NOW` is a fixed local time; every `at` is relative to it.
//
// A fixture names a SNAPSHOT, not a state: the state id is what the model derives from it
// (`tests/macPopoverModel.test.ts` asserts that), except the ones that need a user action first
// (`unlock`, `finderadding`), which the Playwright script drives.

export const NOW = Math.floor(new Date(2026, 9, 1, 12, 0, 0).getTime() / 1000)
const MIN = 60
const HOUR = 3600
const DAY = 86400

export const ME = 'sam@example.eu'
const GB = 1_000_000_000

const base = () => ({
  phase: 'synced',
  generated_at: NOW,
  account: { email: ME, logged_in: true, vault_unlocked: true, auth_expired: false },
  paused: false,
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: NOW - 2 * MIN },
  reason: null,
  finder: { setup: 'ready', reason: null, reason_line: null },
  storage: { used_bytes: 84.3 * GB, quota_bytes: 200 * GB, fetched_at: NOW - 60, stale: false },
  storage_full: false,
  pending_changes: 0,
  conflicts: { count: 0, files: [] },
  activity: [],
})

const row = (id, name, folder, direction, state, ago, extra = {}) => ({
  id, name, folder, path: `${folder}/${name}`, direction, state, at: NOW - ago, size_bytes: 1000, done_bytes: null, total_bytes: null, ...extra,
})

export const RECENT = [
  row('r1', 'Q3 ledger reconciliation.xlsx', 'Investigations', 'up', 'done', 2 * MIN),
  row('r2', 'Source documents, batch 12.pdf', 'Ledger gap', 'up', 'done', 14 * MIN),
  row('r3', 'IMG_4821.HEIC', 'Photos', 'down', 'done', HOUR),
  row('r4', 'Board notes, 29 Sept.md', 'Team shared', 'down', 'done', 20 * HOUR),
  row('r5', 'Interview 07, transcript.docx', 'Investigations', 'up', 'done', 21 * HOUR),
]

const active = (withBytes) => [
  row('a1', 'Interview 09, raw audio.wav', 'Investigations', 'up', 'active', 5, withBytes ? { done_bytes: 62, total_bytes: 100 } : {}),
  row('a2', 'Site visit, north wing.mov', 'Photos', 'up', 'active', 6, withBytes ? { done_bytes: 21, total_bytes: 100 } : {}),
  row('a3', 'Board notes, 30 Sept.md', 'Team shared', 'down', 'active', 7, withBytes ? { done_bytes: 88, total_bytes: 100 } : {}),
]

const merge = (patch) => ({ ...base(), ...patch })

/** One snapshot per state id (the design's ids). `unlock` and `finderadding` reuse the snapshot of the state they start from. */
export const SNAPSHOTS = {
  synced: merge({ activity: RECENT }),
  conflict: merge({ conflicts: { count: 1, files: [{ file_id: 'f1', file_name: 'Budget 2027.xlsx' }] }, activity: RECENT.slice(0, 4) }),
  empty: merge({ engine: { ...base().engine, last_tick_ok_at: NOW - 5 } }),
  syncing: merge({
    phase: 'syncing',
    engine: { state: 'syncing', files_remaining: 14, bytes_total: 1.2 * GB, bytes_done: 0.456 * GB, last_tick_ok_at: NOW - 20 },
    activity: [...active(true), RECENT[0]],
  }),
  syncing0: merge({
    phase: 'syncing',
    engine: { state: 'syncing', files_remaining: 14, bytes_total: null, bytes_done: null, last_tick_ok_at: NOW - 20 },
    activity: [...active(false), RECENT[0]],
  }),
  locked: merge({ phase: 'locked', account: { email: ME, logged_in: true, vault_unlocked: false, auth_expired: false }, storage: null, engine: { ...base().engine, state: 'stopped', last_tick_ok_at: null } }),
  paused: merge({ phase: 'paused', paused: true, pending_changes: 3 }),
  error: merge({ phase: 'error', engine: { ...base().engine, state: 'error' }, reason: { code: 'timeout', detail: 'timeout after 30 s' } }),
  offline: merge({ phase: 'offline', engine: { ...base().engine, state: 'offline' }, reason: { code: 'connect', detail: null } }),
  storage: merge({ phase: 'storage_full', storage_full: true, pending_changes: 3, storage: { used_bytes: 200 * GB, quota_bytes: 200 * GB, fetched_at: NOW - 60, stale: false } }),
  signedout: merge({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false, auth_expired: false }, storage: null }),
  ended: merge({ phase: 'session_ended', account: { email: ME, logged_in: true, vault_unlocked: false, auth_expired: true }, storage: null }),
  finder: merge({ phase: 'finder_missing', finder: { setup: 'missing', reason: null, reason_line: null } }),
  finderfail: merge({ phase: 'finder_failed', finder: { setup: 'failed', reason: 'timeout', reason_line: 'reason: timeout' } }),
  finderoff: merge({ phase: 'finder_user_disabled', finder: { setup: 'user_disabled', reason: 'user_disabled', reason_line: 'reason: user_disabled' } }),
}

export const base_snapshot = base
