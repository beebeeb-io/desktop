// What Settings › Sync and the Windows tray popover say about sync (0.8.15). The fixture is the JSON the Rust side
// serialises (`sync_activity::tests::the_summary_is_the_json_the_frontend_parses` compares against the same file), so
// a field renamed on one side fails one of the two tests.
process.env.TZ = 'Europe/Amsterdam'

import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { SyncStatus } from '../src/desktopApi'
import {
  gaveUpSentence,
  heldSentence,
  parseSyncActivitySummary,
  shortDate,
  syncActivityView,
  type SyncActivitySummary,
} from '../src/windows/syncActivityModel'

const fixture: unknown = JSON.parse(readFileSync(new URL('./fixtures/sync-activity-summary.on-hold.json', import.meta.url), 'utf8'))
const now = new Date('2026-10-26T12:00:00+01:00')
const signedIn: SyncStatus = { logged_in: true, engine: 'running', sync_root: 'C:\\Users\\Fixture\\Beebeeb', syncing: 0, cloud_only: 0, conflicts: 0 }

function summary(patch: Partial<SyncActivitySummary> = {}): SyncActivitySummary {
  return {
    paused: false,
    engine_state: 'idle',
    files_remaining: null,
    transfers_active: 0,
    last_tick_ok_at: 1_793_000_000,
    queue: { waiting: 0, gave_up: 0, held: [] },
    startup_check: null,
    ...patch,
  }
}

/** Guus's PC on 26 Oct: 2,924 changes paused for `auth` on 4 Oct, 43 that gave up, nothing moving. */
function guusOnHold(patch: Partial<SyncActivitySummary> = {}): SyncActivitySummary {
  const parsed = parseSyncActivitySummary(fixture)
  if (!parsed) throw new Error('fixture did not parse')
  return { ...parsed, engine_state: 'idle', startup_check: null, ...patch }
}

const HELD_LINE = '2,924 changes from 4 Oct are on hold. An update will repair them.'
const GAVE_UP_LINE = '43 changes stopped after repeated errors and are not retried.'

describe('the summary contract', () => {
  test('the Rust fixture parses, field for field', () => {
    expect(parseSyncActivitySummary(fixture)).toEqual({
      paused: false,
      engine_state: 'running',
      files_remaining: null,
      transfers_active: 0,
      last_tick_ok_at: null,
      queue: { waiting: 0, gave_up: 43, held: [{ reason: 'auth', count: 2924, oldest_at: 1_791_115_500, newest_at: 1_791_122_220 }] },
      startup_check: { done: 12_400, total: 37_169 },
    })
  })

  test('anything outside the shape is no summary, never a guess', () => {
    const good = summary()
    for (const bad of [
      null,
      [],
      'paused',
      { ...good, paused: 'no' },
      { ...good, engine_state: undefined },
      { ...good, transfers_active: -1 },
      { ...good, transfers_active: 1.5 },
      { ...good, files_remaining: '3' },
      { ...good, queue: { waiting: 0, gave_up: 0 } },
      { ...good, queue: { waiting: 0, gave_up: -2, held: [] } },
      { ...good, queue: { waiting: 0, gave_up: 0, held: [{ reason: 7, count: 1, oldest_at: 0, newest_at: 0 }] } },
      { ...good, startup_check: { done: 1 } },
    ]) {
      expect(parseSyncActivitySummary(bad)).toBeNull()
    }
    expect(parseSyncActivitySummary(good)).toEqual(good)
  })
})

describe('Guus’s on-hold state', () => {
  test('is shown as on hold with both counts, never as synced or syncing', () => {
    const view = syncActivityView(signedIn, guusOnHold(), { now })
    expect(view.kind).toBe('on_hold')
    expect(view.label).toBe('On hold')
    expect(view.headline).toBe(HELD_LINE)
    expect(view.details).toEqual([GAVE_UP_LINE])
    expect(view.tone).toBe('attention')
    expect(JSON.stringify(view)).not.toMatch(/synced|syncing/i)
  })

  test('nothing offers to resume or retry the held changes (Mac lead ruling: they stay paused until an update)', () => {
    for (const s of [guusOnHold(), guusOnHold({ paused: true }), guusOnHold({ engine_state: 'running', startup_check: { done: 1, total: 2 } })]) {
      const words = [syncActivityView(signedIn, s, { now }).headline, ...syncActivityView(signedIn, s, { now }).details].join(' ')
      expect(words).not.toMatch(/\bresume them\b|\bunpause\b|\btry again\b|\bretry now\b/i)
    }
  })

  test('during the startup check, the check is the headline with its progress, and the held counts stay visible', () => {
    const view = syncActivityView(signedIn, parseSyncActivitySummary(fixture), { now })
    expect(view.kind).toBe('checking')
    expect(view.headline).toBe('Checking files on this PC: 12,400 of 37,169.')
    expect(view.progress).toEqual({ done: 12_400, total: 37_169 })
    expect(view.details).toEqual([HELD_LINE, GAVE_UP_LINE])
    expect(view.tone).toBe('busy')
  })

  test('paused: the pause is the headline, and the held counts still show', () => {
    const view = syncActivityView(signedIn, guusOnHold({ paused: true, queue: { waiting: 3, gave_up: 43, held: guusOnHold().queue!.held } }), { now })
    expect(view.kind).toBe('paused')
    expect(view.paused).toBe(true)
    expect(view.headline).toBe('Sync is paused.')
    expect(view.details).toEqual(['3 changes wait until you resume.', HELD_LINE, GAVE_UP_LINE])
  })
})

describe('never calm over pending work', () => {
  test('a stale `syncing` count with nothing on the transfer board is not "Syncing"', () => {
    const view = syncActivityView({ ...signedIn, syncing: 5 }, summary(), { now })
    expect(view.kind).toBe('up_to_date')
  })

  test('work on the transfer board is "Syncing", with the engine’s count when it gave one', () => {
    expect(syncActivityView(signedIn, summary({ transfers_active: 2 }), { now }).headline).toBe('2 files syncing.')
    expect(syncActivityView(signedIn, summary({ engine_state: 'syncing', files_remaining: 1 }), { now }).headline).toBe('1 file syncing.')
    expect(syncActivityView(signedIn, summary({ engine_state: 'syncing', files_remaining: 0 }), { now }).headline).toBe('Syncing.')
  })

  test('changes waiting in the queue are "waiting", not "up to date"', () => {
    const view = syncActivityView(signedIn, summary({ queue: { waiting: 1, gave_up: 0, held: [] } }), { now })
    expect(view.kind).toBe('waiting')
    expect(view.headline).toBe('1 change waiting to sync.')
    expect(view.tone).toBe('busy')
  })

  test('before the engine’s first check it is "Starting", not "up to date"', () => {
    const view = syncActivityView(signedIn, summary({ engine_state: 'running', last_tick_ok_at: null }), { now })
    expect(view.kind).toBe('starting')
    expect(view.tone).not.toBe('good')
  })

  test('without the summary there is no "up to date": not read yet, or unreadable', () => {
    expect(syncActivityView(signedIn, undefined, { now }).kind).toBe('reading')
    const unreadable = syncActivityView(signedIn, null, { now })
    expect(unreadable.kind).toBe('unconfirmed')
    expect(unreadable.headline).toBe('Sync is running. What is queued could not be read.')
    for (const view of [syncActivityView(signedIn, undefined, { now }), unreadable, syncActivityView(null, undefined, { now })]) {
      expect(view.tone).not.toBe('good')
    }
  })

  test('up to date only with an empty queue, nothing moving and a finished check', () => {
    const view = syncActivityView(signedIn, summary(), { now })
    expect(view.kind).toBe('up_to_date')
    expect(view.headline).toBe('Your files are synced.')
    expect(view.tone).toBe('good')
    expect(view.details).toEqual([])
  })
})

describe('precedence', () => {
  test('signed out, session ended and a refused engine come before any count', () => {
    expect(syncActivityView({ ...signedIn, logged_in: false }, guusOnHold(), { now }).kind).toBe('signed_out')
    expect(syncActivityView({ ...signedIn, auth_expired: true }, guusOnHold(), { now }).headline).toBe('Signed out on this PC. Sign in again to sync.')
    const refused = syncActivityView({ ...signedIn, engine_refusal: { code: 'identity_unknown', sentence: 'Fixture refusal sentence.' } }, guusOnHold(), { now })
    expect(refused.kind).toBe('refused')
    expect(refused.headline).toBe('Fixture refusal sentence.')
    expect(refused.details).toEqual([HELD_LINE, GAVE_UP_LINE])
  })

  test('the pause flag falls back to the saved setting only when the summary is missing', () => {
    expect(syncActivityView(signedIn, null, { pausedFallback: true, now }).kind).toBe('paused')
    expect(syncActivityView(signedIn, summary({ paused: false }), { pausedFallback: true, now }).kind).toBe('up_to_date')
  })

  test('Pause / Resume applies only while signed in', () => {
    expect(syncActivityView(signedIn, summary(), { now }).pausable).toBe(true)
    expect(syncActivityView({ ...signedIn, logged_in: false }, summary(), { now }).pausable).toBe(false)
    expect(syncActivityView(null, summary(), { now }).pausable).toBe(false)
  })

  test('offline and a failed check say so, with the queue under them', () => {
    const offline = syncActivityView(signedIn, guusOnHold({ engine_state: 'offline' }), { now })
    expect(offline.kind).toBe('offline')
    expect(offline.details).toEqual([HELD_LINE, GAVE_UP_LINE])
    expect(syncActivityView(signedIn, summary({ engine_state: 'error' }), { now }).kind).toBe('error')
  })

  test('the device noun follows the platform', () => {
    expect(syncActivityView(signedIn, summary({ startup_check: { done: 0, total: 1 } }), { device: 'this device', now }).headline).toBe('Checking files on this device: 0 of 1.')
  })
})

describe('words', () => {
  test('singular and plural', () => {
    expect(heldSentence({ reason: 'auth', count: 1, oldest_at: 1_791_115_500, newest_at: 1_791_115_500 }, now)).toBe('1 change from 4 Oct is on hold. An update will repair it.')
    expect(gaveUpSentence(1)).toBe('1 change stopped after repeated errors and is not retried.')
    expect(gaveUpSentence(1_200)).toBe('1,200 changes stopped after repeated errors and are not retried.')
  })

  test('dates: one day is "from", a span is "since", another year carries the year, none is left out', () => {
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: 1_791_115_500, newest_at: 1_791_115_500 + 3 * 86_400 }, now)).toBe('2 changes since 4 Oct are on hold. An update will repair them.')
    expect(shortDate(1_791_115_500 - 365 * 86_400, now)).toBe('4 Oct 2025')
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: 0, newest_at: 0 }, now)).toBe('2 changes are on hold. An update will repair them.')
  })

  test('every pause reason has its own sentence, and only auth promises a repair', () => {
    const at = { oldest_at: 1_791_115_500, newest_at: 1_791_115_500 }
    expect(heldSentence({ reason: 'quota', count: 3, ...at }, now)).toBe('3 changes are on hold because your storage is full.')
    expect(heldSentence({ reason: 'permission', count: 1, ...at }, now)).toBe('1 change is on hold because you don’t have permission to make it.')
    expect(heldSentence({ reason: 'locked', count: 2, ...at }, now)).toBe('2 changes are on hold while Beebeeb is locked.')
    expect(heldSentence({ reason: 'key_replaced', count: 2, ...at }, now)).toBe('2 changes are on hold: they were made with a key this account no longer uses.')
    expect(heldSentence({ reason: 'other', count: 2, ...at }, now)).toBe('2 changes from 4 Oct are on hold.')
    for (const reason of ['quota', 'permission', 'locked', 'key_replaced', 'other']) {
      expect(heldSentence({ reason, count: 2, ...at }, now)).not.toContain('update')
    }
  })

  test('held groups with no changes are left out', () => {
    const view = syncActivityView(signedIn, summary({ queue: { waiting: 0, gave_up: 0, held: [{ reason: 'quota', count: 0, oldest_at: 0, newest_at: 0 }] } }), { now })
    expect(view.kind).toBe('up_to_date')
  })
})
