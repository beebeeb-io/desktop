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

const HELD_LINE = '2,924 changes from 4 Oct are on hold until you sign in again.'
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
  // Coordinator ruling (26 Oct): with the engine running, the state word is "Not in sync" (2,924 changes are not), not
  // "On hold", which reads as if sync were paused while new changes do flow. The notice keeps the held sentence.
  test('is "Not in sync" with both counts as the notice, never synced, syncing or on hold', () => {
    const view = syncActivityView(signedIn, guusOnHold(), { now })
    expect(view.kind).toBe('not_in_sync')
    expect(view.label).toBe('Not in sync')
    expect(view.headline).toBe(HELD_LINE)
    expect(view.details).toEqual([GAVE_UP_LINE])
    expect(view.tone).toBe('attention')
    expect(JSON.stringify(view)).not.toMatch(/synced|syncing/i)
  })

  test('the state word is "Paused" only for a user pause, "Syncing" while work moves, never "On hold"', () => {
    const label = (s: SyncActivitySummary, status: SyncStatus = signedIn) => syncActivityView(status, s, { now }).label
    expect(label(guusOnHold())).toBe('Not in sync')
    expect(label(guusOnHold({ queue: { waiting: 0, gave_up: 43, held: [] } }))).toBe('Not in sync')
    expect(label(guusOnHold({ paused: true }))).toBe('Paused')
    const moving = syncActivityView(signedIn, guusOnHold({ transfers_active: 2 }), { now })
    expect(moving.label).toBe('Syncing')
    expect(moving.details).toEqual([HELD_LINE, GAVE_UP_LINE])
    expect(label(guusOnHold({ engine_state: 'syncing', files_remaining: 4 }))).toBe('Syncing')
    for (const s of [
      guusOnHold(),
      guusOnHold({ engine_state: 'running' }),
      guusOnHold({ engine_state: 'offline' }),
      guusOnHold({ engine_state: 'error' }),
      guusOnHold({ queue: { ...guusOnHold().queue!, waiting: 3 } }),
      guusOnHold({ startup_check: { done: 1, total: 2 } }),
    ]) {
      expect(label(s)).not.toMatch(/on hold|paused/i)
    }
  })

  test('nothing offers to resume or retry the held changes (the engine resumes them after a sign-in, spec 1888 D11)', () => {
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

describe('the start-up guard’s pass (engine lane: `startup_guard`, live on `sync_status`)', () => {
  const live = (running: boolean, done: number, total: number) => ({ ...signedIn, startup_guard: { running, done, total } })

  test('the live value on the status decides, over the last pass the summary heard of', () => {
    expect(syncActivityView(live(true, 5, 10), summary(), { now }).headline).toBe('Checking files on this PC: 5 of 10.')
    const atRest = syncActivityView(live(false, 0, 0), summary({ startup_check: { done: 12_400, total: 37_169 } }), { now })
    expect(atRest.kind).toBe('up_to_date')
    expect(atRest.progress).toBeNull()
  })

  test('without it on the status, the summary’s pass is shown', () => {
    expect(syncActivityView(signedIn, summary({ startup_check: { done: 1, total: 4 } }), { now }).kind).toBe('checking')
  })

  test('a live value outside the shape is no check', () => {
    for (const bad of [{ running: 'yes', done: 1, total: 2 }, { running: true, done: -1, total: 2 }, { running: true, done: 0, total: 0 }, 'running']) {
      const status = { ...signedIn, startup_guard: bad } as unknown as SyncStatus
      expect(syncActivityView(status, summary({ startup_check: { done: 1, total: 4 } }), { now }).kind).toBe('up_to_date')
    }
  })

  test('uploads during the pass are the headline, with the pass and its progress under them', () => {
    const view = syncActivityView(live(true, 2_000, 37_169), guusOnHold({ transfers_active: 3 }), { now })
    expect(view.kind).toBe('syncing')
    expect(view.headline).toBe('3 files syncing.')
    expect(view.details).toEqual(['Checking files on this PC: 2,000 of 37,169.', HELD_LINE, GAVE_UP_LINE])
    expect(view.progress).toEqual({ done: 2_000, total: 37_169 })
  })

  test('paused during the pass: the pause is the headline and the pass stays in view', () => {
    const view = syncActivityView(live(true, 2_000, 37_169), summary({ paused: true }), { now })
    expect(view.kind).toBe('paused')
    expect(view.details).toEqual(['Checking files on this PC: 2,000 of 37,169.'])
    expect(view.progress).toEqual({ done: 2_000, total: 37_169 })
  })

  test('conflicts that are not the headline are a detail line', () => {
    const view = syncActivityView({ ...live(true, 1, 2), conflicts: 2 }, summary(), { now })
    expect(view.kind).toBe('checking')
    expect(view.details).toEqual(['2 conflicts to review.'])
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
    expect(heldSentence({ reason: 'auth', count: 1, oldest_at: 1_791_115_500, newest_at: 1_791_115_500 }, now)).toBe('1 change from 4 Oct is on hold until you sign in again.')
    expect(gaveUpSentence(1)).toBe('1 change stopped after repeated errors and is not retried.')
    expect(gaveUpSentence(1_200)).toBe('1,200 changes stopped after repeated errors and are not retried.')
  })

  test('dates: one day is "from", a span is "since", another year carries the year, none is left out', () => {
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: 1_791_115_500, newest_at: 1_791_115_500 + 3 * 86_400 }, now)).toBe('2 changes since 4 Oct are on hold until you sign in again.')
    expect(shortDate(1_791_115_500 - 365 * 86_400, now)).toBe('4 Oct 2025')
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: 0, newest_at: 0 }, now)).toBe('2 changes are on hold until you sign in again.')
  })

  test('every pause reason has its own sentence, and none promises an update', () => {
    const at = { oldest_at: 1_791_115_500, newest_at: 1_791_115_500 }
    expect(heldSentence({ reason: 'quota', count: 3, ...at }, now)).toBe('3 changes are on hold because your storage is full.')
    expect(heldSentence({ reason: 'permission', count: 1, ...at }, now)).toBe('1 change is on hold because you don’t have permission to make it.')
    expect(heldSentence({ reason: 'locked', count: 2, ...at }, now)).toBe('2 changes are on hold while Beebeeb is locked.')
    expect(heldSentence({ reason: 'key_replaced', count: 2, ...at }, now)).toBe('2 changes are on hold: they were made with a key this account no longer uses.')
    expect(heldSentence({ reason: 'other', count: 2, ...at }, now)).toBe('2 changes from 4 Oct are on hold.')
    for (const reason of ['auth', 'needs_review', 'waiting_for_folder', 'quota', 'permission', 'locked', 'key_replaced', 'found_while_paused', 'other']) {
      expect(heldSentence({ reason, count: 2, ...at }, now)).not.toContain('update')
    }
  })

  // Task 2267: items Beebeeb could not prove were the account's own wait for the person's decision in Settings › Sync.
  test('found while paused: its own sentence, singular and plural, with no date and no promise', () => {
    const at = { oldest_at: 1_791_115_500, newest_at: 1_791_115_500 + 3 * 86_400 }
    expect(heldSentence({ reason: 'found_while_paused', count: 1, ...at }, now)).toBe('1 change found while sync was paused is not uploaded yet.')
    expect(heldSentence({ reason: 'found_while_paused', count: 3, ...at }, now)).toBe('3 changes found while sync was paused are not uploaded yet.')
    expect(heldSentence({ reason: 'found_while_paused', count: 1_200, oldest_at: 0, newest_at: 0 }, now)).toBe('1,200 changes found while sync was paused are not uploaded yet.')
  })

  test('found while paused, alone in the queue: "Not in sync" with its sentence as the notice', () => {
    const view = syncActivityView(signedIn, summary({ queue: { waiting: 0, gave_up: 0, held: [{ reason: 'found_while_paused', count: 3, oldest_at: 0, newest_at: 0 }] } }), { now })
    expect(view.kind).toBe('not_in_sync')
    expect(view.headline).toBe('3 changes found while sync was paused are not uploaded yet.')
    expect(view.tone).toBe('attention')
  })

  test('held groups with no changes are left out', () => {
    const view = syncActivityView(signedIn, summary({ queue: { waiting: 0, gave_up: 0, held: [{ reason: 'quota', count: 0, oldest_at: 0, newest_at: 0 }] } }), { now })
    expect(view.kind).toBe('up_to_date')
  })
})

// Spec 1888/2226 D13: what is held, and why, in the words the person reads. The date is the day the oldest held change
// was queued (`oldest_at`), never hard-coded; 0 (not recorded) drops the date.
describe('held sentences for auth, needs_review, waiting_for_folder', () => {
  const OCT4 = 1_791_115_500
  const OCT27 = OCT4 + 23 * 86_400
  const oct28 = new Date('2026-10-28T12:00:00+01:00')

  test('auth: the oldest change’s day, until a sign-in', () => {
    expect(heldSentence({ reason: 'auth', count: 2924, oldest_at: OCT4, newest_at: OCT4 }, now)).toBe('2,924 changes from 4 Oct are on hold until you sign in again.')
  })

  test('auth: the date comes from oldest_at, never a fixed day', () => {
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: OCT27, newest_at: OCT27 }, oct28)).toBe('2 changes from 27 Oct are on hold until you sign in again.')
  })

  test('auth: no recorded date, no date phrase', () => {
    expect(heldSentence({ reason: 'auth', count: 2, oldest_at: 0, newest_at: 0 }, now)).toBe('2 changes are on hold until you sign in again.')
  })

  test('needs_review: what happened and what the person can do, singular and plural, with no date', () => {
    expect(heldSentence({ reason: 'needs_review', count: 3, oldest_at: OCT4, newest_at: OCT27 }, oct28)).toBe(
      "3 changes are on hold: an item with the same name is already in Beebeeb. They weren't sent, and nothing was overwritten or deleted. Contact support@beebeeb.io and we'll sort it out with you.",
    )
    expect(heldSentence({ reason: 'needs_review', count: 1, oldest_at: 0, newest_at: 0 }, now)).toBe(
      "1 change is on hold: an item with the same name is already in Beebeeb. It wasn't sent, and nothing was overwritten or deleted. Contact support@beebeeb.io and we'll sort it out with you.",
    )
  })

  test('waiting_for_folder: singular and plural, with no date', () => {
    expect(heldSentence({ reason: 'waiting_for_folder', count: 1, oldest_at: OCT4, newest_at: OCT4 }, now)).toBe('1 change is waiting for its folder to reach Beebeeb.')
    expect(heldSentence({ reason: 'waiting_for_folder', count: 1_200, oldest_at: 0, newest_at: 0 }, now)).toBe('1,200 changes are waiting for their folder to reach Beebeeb.')
  })
})
