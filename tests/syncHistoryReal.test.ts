// Sync history, real: the flyout's finished rows (popover_snapshot), the queue's problems (drive_sync_problems) and
// its counts (sync_activity_summary), merged inside the 30-day window. A problem row carries its reason and wins over
// the flyout's copy of the same failure; the count at the top is the rows the Problems filter shows.
// Run by `bun test tests/syncHistoryReal.test.ts`. No production module imports this file.
import { describe, expect, test } from 'bun:test'
import type { ActivityRow } from '../src/popoverContract'
import type { SyncActivitySummary } from '../src/windows/syncActivityModel'
import type { SyncProblem } from '../src/v2/source/extras'
import { applyFilter, problemCount } from '../src/v2/sections/sync-history/model'
import * as model from '../src/v2/sections/sync-history/model'

const NOW = new Date('2026-11-09T12:00:00.000Z')
const at = (secondsAgo: number) => NOW.getTime() / 1000 - secondsAgo

/** popover_snapshot's activity row, as the Rust command serializes it. */
function activity(p: Partial<ActivityRow> & Pick<ActivityRow, 'id' | 'name' | 'state'>): ActivityRow {
  return {
    folder: 'Docs', path: `Docs/${p.name}`, direction: p.state === 'failed' ? null : 'up',
    at: at(3_600), size_bytes: 12, done_bytes: 12, total_bytes: 12, ...p,
  }
}

/** drive_sync_problems' row, as the Rust command serializes it. */
function problem(p: Partial<SyncProblem> & Pick<SyncProblem, 'id' | 'name' | 'reason'>): SyncProblem {
  return { folder: 'Docs', retryable: true, backup: false, ...p }
}

const summary = (gaveUp: number | null): SyncActivitySummary => ({
  paused: false, engine_state: 'idle', files_remaining: 0, transfers_active: 0, last_tick_ok_at: null,
  queue: gaveUp === null ? null : { waiting: 2, gave_up: gaveUp, held: [], not_downloaded: 0 },
  startup_check: null,
})

describe('sync history, from the commands', () => {
  test('a problem row keeps its reason once and its change id for Try again', () => {
    const rows = model.rowsFromProblems([
      problem({ id: 'op-1', name: 'Tram 28.mov', folder: 'Lisbon trip', reason: 'Couldn’t reach Beebeeb after 5 tries.', at: '2026-11-08T08:20:00Z' }),
      problem({ id: 'op-2', name: 'Notes.md', reason: 'It didn’t sync. Nothing was lost.', backup: false }),
      problem({ id: 'op-3', name: 'Photos', folder: '', reason: 'Stopped after 5 tries.', backup: true }),
    ], NOW)

    expect(rows.map((r) => r.problem)).toEqual([
      'Couldn’t reach Beebeeb after 5 tries. Nothing was lost.',
      'It didn’t sync. Nothing was lost.',
      'Stopped after 5 tries. Nothing was lost.',
    ])
    expect(rows.map((r) => r.what)).toEqual(['Didn’t sync · Lisbon trip', 'Didn’t sync · Docs', 'Didn’t back up'])
    expect(rows[2].at).toBe(NOW.toISOString())
    expect(rows.map((r) => model.problemId(r))).toEqual(['op-1', 'op-2', 'op-3'])
  })

  test('the flyout rows: moving ones give way, a failed one says nothing was lost, a transfer has no item', () => {
    const rows = model.rowsFromActivity([
      activity({ id: 'i-live', name: 'Moving.mov', state: 'active' }),
      activity({ id: 'i-up', name: 'Plan.pdf', state: 'done', direction: 'up', folder: 'Contracts', at: at(60) }),
      activity({ id: 'transfer-7', name: 'Big.iso', state: 'done', direction: 'down', folder: '', at: at(120) }),
      activity({ id: 'i-bad', name: 'Tram 28.mov', state: 'failed', folder: 'Lisbon trip', at: at(180) }),
    ])

    expect(rows.map((r) => r.id)).toEqual(['i-up', 'transfer-7', 'i-bad'])
    expect(rows.map((r) => r.what)).toEqual(['Uploaded to Contracts', 'Downloaded from Beebeeb', 'Didn’t sync · Lisbon trip'])
    expect(rows[1].itemId).toBeUndefined()
    expect(rows[2].problem).toBe('It didn’t sync. Nothing was lost; Beebeeb will try again.')
  })

  test('the merge: a problem row wins over the flyout copy, an unrelated failure stays, newest first', () => {
    const merged = model.mergeHistory([
      // The same three failures the queue explains, plus one the queue did not list.
      activity({ id: 'h1', name: 'Studio walkthrough.mov', state: 'failed', folder: 'Video', at: at(300) }),
      activity({ id: 'h4', name: 'Tram 28.mov', state: 'failed', folder: 'Lisbon trip', at: at(600) }),
      activity({ id: 'h11', name: 'Budget 2026 (2).xlsx', state: 'failed', folder: 'Finance', at: at(900) }),
      activity({ id: 'h-orphan', name: 'Orphan.bin', state: 'failed', folder: 'Docs', at: at(1_200) }),
      activity({ id: 'h2', name: 'Contract draft.pdf', state: 'done', at: at(1_500) }),
    ], [
      problem({ id: 'h1', name: 'Studio walkthrough.mov', folder: 'Video', reason: 'It’s open in another app.', at: '2026-11-09T11:55:00Z' }),
      problem({ id: 'h4', name: 'Tram 28.mov', folder: 'Lisbon trip', reason: 'Couldn’t reach Beebeeb after 5 tries.', at: '2026-11-09T11:50:00Z' }),
      problem({ id: 'h11', name: 'Budget 2026 (2).xlsx', folder: 'Finance', reason: 'The name is already in use.', at: '2026-11-09T11:45:00Z' }),
    ], NOW)

    // Three problems (with their reasons) and the one failure the queue did not explain; no duplicates.
    expect(merged).toHaveLength(5)
    expect(merged.map((r) => r.id)).toEqual(['problem:h1', 'problem:h4', 'problem:h11', 'h-orphan', 'h2'])
    expect(merged.map((r) => r.problem)).toEqual([
      'It’s open in another app. Nothing was lost.',
      'Couldn’t reach Beebeeb after 5 tries. Nothing was lost.',
      'The name is already in use. Nothing was lost.',
      'It didn’t sync. Nothing was lost; Beebeeb will try again.',
      undefined,
    ])
    // The count at the top equals the rows the Problems tab shows.
    expect(problemCount(merged)).toBe(4)
    expect(applyFilter(merged, 'problems', '')).toHaveLength(problemCount(merged))
  })

  test('the gave-up count comes from the queue and stays zero when that command has nothing', () => {
    expect(model.gaveUp(summary(4))).toBe(4)
    expect(model.gaveUp(summary(0))).toBe(0)
    expect(model.gaveUp(summary(null))).toBe(0)
    expect(model.gaveUp(null)).toBe(0)
  })

  test('the window uses the passed now; the route asks for Problems or All', () => {
    const rows = model.rowsFromActivity([
      activity({ id: 'fresh', name: 'fresh.txt', state: 'done', at: at(0) }),
      activity({ id: 'last', name: 'last.txt', state: 'done', at: NOW.getTime() / 1000 - 30 * 86_400 }),
      activity({ id: 'old', name: 'old.txt', state: 'done', at: NOW.getTime() / 1000 - 30 * 86_400 - 1 }),
    ])
    const shown = model.withinWindow(rows, NOW)

    expect(shown.map((r) => r.id)).toEqual(['fresh', 'last'])
    expect(shown[0].at > shown[1].at).toBe(true)
    expect(model.routeFilter('problems')).toBe('problems')
    expect(model.routeFilter(null)).toBe('all')
    expect(model.routeFilter(undefined)).toBe('all')
    expect(model.routeFilter('')).toBe('all')
  })
})
