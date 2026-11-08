// The board's flyout states as data (B09's own fixtures): a PopoverSnapshot per state, in the board's words.
import type { ActivityRow, PopoverSnapshot } from '../../popoverContract'
import type { FlyoutExtras } from './model'

export const NOW = 1_794_000_000 // a fixed "now": the board's 2 min, 44 min, 2 h are offsets from it

const row = (id: string, name: string, folder: string, direction: 'up' | 'down' | null, ago: number, extra: Partial<ActivityRow> = {}): ActivityRow => ({
  id, name, folder, path: `${folder}/${name}`, direction, state: 'done', at: NOW - ago, size_bytes: 120_000, done_bytes: null, total_bytes: null, ...extra,
})

const NAMES: [string, string, 'up' | 'down'][] = [
  ['Contract draft.pdf', 'Contracts', 'up'], ['Budget 2026.xlsx', 'Finance', 'up'], ['Team photo.jpg', 'Photos', 'down'],
  ['Roadmap.pdf', 'Planning', 'up'], ['Notes.md', 'Notes', 'up'], ['Logo.svg', 'Design', 'down'], ['Invoice 0412.pdf', 'Invoices', 'up'],
  ['Meeting recording.m4a', 'Meetings', 'down'], ['Brand guide.pdf', 'Design', 'up'], ['Passport scan.jpg', 'Personal', 'up'],
]

export function history(count: number): ActivityRow[] {
  const ago = [120, 44 * 60, 2 * 3600, 5 * 3600, 26 * 3600, 3 * 86400, 5 * 86400, 9 * 86400]
  return Array.from({ length: count }, (_, i) => {
    const [name, folder, dir] = NAMES[i % NAMES.length]
    const label = i < NAMES.length ? name : name.replace(/(\.[^.]+)$/, ` (${Math.floor(i / NAMES.length) + 1})$1`)
    return row(`h${i}`, label, folder, dir, ago[Math.min(i, ago.length - 1)] + (i >= ago.length ? (i - ago.length) * 3600 : 0))
  })
}

export function snapshot(patch: Partial<PopoverSnapshot> = {}): PopoverSnapshot {
  return {
    phase: 'synced', sync_phase: 'synced', generated_at: NOW,
    account: { email: 'anna@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false },
    paused: false, engine: { state: 'idle', files_remaining: null, bytes_total: null, bytes_done: null, last_tick_ok_at: NOW - 30 },
    reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
    storage: { used_bytes: 312_000_000_000, quota_bytes: 1_000_000_000_000, fetched_at: NOW, stale: false }, storage_full: false,
    pending_changes: 0, conflicts: { count: 0, files: [] }, activity: history(12), ...patch,
  }
}

export interface FlyoutScenario { snapshot: PopoverSnapshot; extras: FlyoutExtras; scrollToEnd?: boolean }

const base = { totalFiles: 1284 }

export const SCENARIOS: Record<string, FlyoutScenario> = {
  'up-to-date': { snapshot: snapshot(), extras: base },
  syncing: {
    snapshot: snapshot({
      phase: 'syncing', sync_phase: 'syncing', engine: { state: 'syncing', files_remaining: 12, bytes_total: 120_000_000, bytes_done: 72_000_000, last_tick_ok_at: NOW },
      activity: [
        row('a1', 'Contract draft.pdf', 'Contracts', 'up', 0, { state: 'active', done_bytes: 6_400_000, total_bytes: 10_000_000 }),
        row('a2', 'Team photo.jpg', 'Photos', 'down', 0, { state: 'active', done_bytes: 1_200_000, total_bytes: 4_000_000 }),
        row('a3', 'Roadmap.pdf', 'Planning', 'up', 0, { state: 'active', done_bytes: 900_000, total_bytes: 8_000_000 }),
        ...history(9),
      ],
    }),
    extras: { ...base, uploadBps: 3_200_000, etaSeconds: 60 },
  },
  paused: { snapshot: snapshot({ phase: 'paused', sync_phase: 'synced', paused: true, pending_changes: 7 }), extras: base },
  'needs-decision': { snapshot: snapshot({ conflicts: { count: 2, files: [{ file_id: 'c1', file_name: 'Budget 2026.xlsx' }, { file_id: 'c2', file_name: 'Roadmap.pdf' }] } }), extras: base },
  offline: { snapshot: snapshot({ phase: 'offline', sync_phase: 'offline' }), extras: base },
  stopped: {
    snapshot: snapshot({
      phase: 'error', sync_phase: 'error',
      activity: [
        row('f1', 'Contract draft.pdf', 'Contracts', null, 600, { state: 'failed' }),
        row('f2', 'Budget 2026.xlsx', 'Finance', null, 700, { state: 'failed' }),
        row('f3', 'Roadmap.pdf', 'Planning', null, 800, { state: 'failed' }),
        ...history(9),
      ],
    }),
    extras: { ...base, stoppedFiles: 3 },
  },
  'almost-full': { snapshot: snapshot({ storage: { used_bytes: 912_000_000_000, quota_bytes: 1_000_000_000_000, fetched_at: NOW, stale: false } }), extras: base },
  'storage-full': {
    snapshot: snapshot({ phase: 'storage_full', sync_phase: 'storage_full', storage_full: true, pending_changes: 9, storage: { used_bytes: 1_000_000_000_000, quota_bytes: 1_000_000_000_000, fetched_at: NOW, stale: false } }),
    extras: base,
  },
  update: { snapshot: snapshot({ update: { version: '0.9.2', channel: null } }), extras: base },
  locked: { snapshot: snapshot({ phase: 'locked', sync_phase: 'synced', account: { email: 'anna@example.eu', logged_in: true, vault_unlocked: false, auth_expired: false } }), extras: base },
  checking: { snapshot: snapshot({ activity: history(6) }), extras: { checking: true } },
  'no-plan': { snapshot: snapshot({ activity: [] }), extras: { noPlan: true } },
  'update-required': { snapshot: snapshot(), extras: { ...base, updateRequired: true } },
  'finder-adding': { snapshot: snapshot({ phase: 'finder_adding', finder: { setup: 'adding', reason: null, reason_line: null }, activity: [] }), extras: base },
  'finder-failed': {
    snapshot: snapshot({ phase: 'finder_failed', finder: { setup: 'failed', reason: '0x8007017C', reason_line: null } }),
    extras: { ...base, finderCode: '0x8007017C' },
  },
  'signed-out': { snapshot: snapshot({ phase: 'session_ended', sync_phase: 'session_ended', account: { email: 'anna@example.eu', logged_in: false, vault_unlocked: false, auth_expired: true } }), extras: base },
  'last-50': { snapshot: snapshot({ activity: history(50) }), extras: base, scrollToEnd: true },
  longest: {
    snapshot: snapshot({
      phase: 'finder_failed', finder: { setup: 'failed', reason: '0x8007017C', reason_line: null },
      activity: [
        row('l1', 'Quarterly results and board presentation, final version after legal review (2).pptx', 'Documents/Company/Board meetings/2026/Q3/Presentations and supporting material', 'up', 120),
        row('l2', 'Scan of the signed tenancy agreement for the Amsterdam office.pdf', 'Documents/Company/Legal/Contracts', 'down', 44 * 60),
        ...history(8),
      ],
    }),
    extras: { ...base, finderCode: '0x8007017C' },
  },
  metered: { snapshot: snapshot(), extras: { ...base, metered: true } },
}
