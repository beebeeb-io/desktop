/**
 * Tests for the downloading / streaming progress row view model (LD-0105 U1).
 *
 * Covers all four UI states (downloading, streaming, buffering, failed), percent
 * and speed formatting, and the invariant that "failed" never says "complete".
 *
 * Mutation check methodology (AGENTS.md): each behaviour is stated as a named
 * expectation that would have failed on the mutant listed in its comment.
 */
import { describe, expect, test } from 'bun:test'
import {
  formatSpeed,
  formatPercent,
  transferUiState,
  traySubline,
  activityDetail,
  progressFraction,
  shouldShowInProgressCard,
  type ActiveTransferRowLike,
} from '../src/windows/downloadProgressModel'

// ── Shared fixtures ───────────────────────────────────────────────────────────

function downloading(overrides: Partial<ActiveTransferRowLike> = {}): ActiveTransferRowLike {
  return { direction: 'down', done: 340, total: 1000, buffered: 0, speedBps: 9_600_000, failed: false, ...overrides }
}
function streaming(overrides: Partial<ActiveTransferRowLike> = {}): ActiveTransferRowLike {
  return { direction: 'stream', done: 0, total: 1000, buffered: 610, speedBps: 9_600_000, failed: false, ...overrides }
}
function buffering(overrides: Partial<ActiveTransferRowLike> = {}): ActiveTransferRowLike {
  return { direction: 'stream', done: 0, total: 0, buffered: 0, speedBps: 8_100_000, failed: false, ...overrides }
}
function failed(overrides: Partial<ActiveTransferRowLike> = {}): ActiveTransferRowLike {
  return { direction: 'down', done: 50, total: 100, buffered: 0, speedBps: 0, failed: true, failedAtSecs: 1_000_000, ...overrides }
}

// ── formatSpeed ───────────────────────────────────────────────────────────────

describe('formatSpeed', () => {
  test('zero speed is "0 B/s"', () => {
    // mutant: removed zero branch → would print "0.0 KB/s"
    expect(formatSpeed(0)).toBe('0 B/s')
  })
  test('negative speed is treated as zero', () => {
    expect(formatSpeed(-1)).toBe('0 B/s')
  })
  test('bytes per second below 1024', () => {
    expect(formatSpeed(512)).toBe('512 B/s')
  })
  test('kilobytes per second', () => {
    expect(formatSpeed(2048)).toBe('2.0 KB/s')
  })
  test('megabytes per second (9.6 MB/s)', () => {
    // mutant: changed divisor from 1_048_576 to 1_000_000 → would print "9.6 MB/s" differently
    expect(formatSpeed(9_600_000)).toBe('9.2 MB/s')
  })
  test('gigabytes per second', () => {
    expect(formatSpeed(2_000_000_000)).toBe('1.9 GB/s')
  })
})

// ── formatPercent ─────────────────────────────────────────────────────────────

describe('formatPercent', () => {
  test('no space before the percent sign', () => {
    // Brand rule: "34%" not "34 %".
    // mutant: added space before % → would return "34 %"
    expect(formatPercent(340, 1000)).toBe('34%')
  })
  test('rounds to nearest whole number', () => {
    expect(formatPercent(1, 3)).toBe('33%')
  })
  test('never exceeds 100', () => {
    // mutant: removed Math.min(100, ...) → could return ">100%"
    expect(formatPercent(1500, 1000)).toBe('100%')
  })
  test('zero total is 0%', () => {
    expect(formatPercent(50, 0)).toBe('0%')
  })
  test('100% at total', () => {
    expect(formatPercent(1000, 1000)).toBe('100%')
  })
})

// ── transferUiState ───────────────────────────────────────────────────────────

describe('transferUiState', () => {
  test('failed row → "failed" regardless of direction', () => {
    // mutant: removed failed check → would return "downloading"
    expect(transferUiState(failed())).toBe('failed')
    expect(transferUiState({ ...streaming(), failed: true })).toBe('failed')
  })
  test('stream with buffered > 0 and total > 0 → "streaming"', () => {
    expect(transferUiState(streaming())).toBe('streaming')
  })
  test('stream with total === 0 → "buffering" (no meaningful percent)', () => {
    // mutant: removed total === 0 check → would return "streaming" with 0/0
    expect(transferUiState(buffering())).toBe('buffering')
  })
  test('stream with buffered === 0 → "buffering"', () => {
    expect(transferUiState({ ...streaming(), buffered: 0 })).toBe('buffering')
  })
  test('down direction → "downloading"', () => {
    expect(transferUiState(downloading())).toBe('downloading')
  })
})

// ── traySubline ───────────────────────────────────────────────────────────────

describe('traySubline', () => {
  test('downloading: "Downloading · 34% · 9.2 MB/s"', () => {
    expect(traySubline(downloading())).toBe('Downloading · 34% · 9.2 MB/s')
  })
  test('streaming: "Streaming · 61% buffered · 9.2 MB/s"', () => {
    expect(traySubline(streaming())).toBe('Streaming · 61% buffered · 9.2 MB/s')
  })
  test('buffering: "Buffering · 7.7 MB/s"', () => {
    expect(traySubline(buffering())).toBe('Buffering · 7.7 MB/s')
  })
  test('failed: short sentence only', () => {
    // mutant: returned full sentence → would exceed the design length budget
    const sub = traySubline(failed())
    expect(sub).toBe("Couldn't finish downloading")
  })
  test('failed: never contains "complete"', () => {
    // mutant: appended "— download complete" → would contain "complete"
    expect(traySubline(failed()).toLowerCase()).not.toContain('complete')
  })
})

// ── activityDetail ────────────────────────────────────────────────────────────

describe('activityDetail', () => {
  test('downloading includes % and speed', () => {
    const d = activityDetail(downloading())
    expect(d).toContain('34%')
    expect(d).toContain('9.2 MB/s')
    expect(d).toContain('Downloading')
  })
  test('streaming includes "buffered"', () => {
    expect(activityDetail(streaming())).toContain('buffered')
  })
  test('failed: ends with a period', () => {
    expect(activityDetail(failed()).endsWith('.')).toBe(true)
  })
  test('failed: never contains "complete"', () => {
    // mutant: "download complete" appended → would contain "complete"
    expect(activityDetail(failed()).toLowerCase()).not.toContain('complete')
  })
})

// ── progressFraction ─────────────────────────────────────────────────────────

describe('progressFraction', () => {
  test('failed → null (no progress bar shown)', () => {
    // mutant: returned 0 instead of null → bar would render at 0 width
    expect(progressFraction(failed())).toBeNull()
  })
  test('downloading: done/total fraction', () => {
    expect(progressFraction(downloading())).toBeCloseTo(0.34)
  })
  test('streaming: buffered/total fraction', () => {
    expect(progressFraction(streaming())).toBeCloseTo(0.61)
  })
  test('buffering (total === 0): null', () => {
    expect(progressFraction(buffering())).toBeNull()
  })
  test('never exceeds 1', () => {
    // mutant: removed Math.min(1, ...) → could return > 1
    expect(progressFraction({ ...downloading(), done: 2000 })).toBe(1)
  })
})

// ── shouldShowInProgressCard ──────────────────────────────────────────────────

describe('shouldShowInProgressCard', () => {
  const NOW = 1_700_000_000_000 // fixed epoch ms for determinism

  test('active (not failed) transfer → show card', () => {
    expect(shouldShowInProgressCard([downloading()], NOW)).toBe(true)
  })
  test('failed within 15 min → show card', () => {
    // 10 min ago
    const tenMinAgo = Math.floor(NOW / 1000) - 600
    expect(shouldShowInProgressCard([failed({ failedAtSecs: tenMinAgo })], NOW)).toBe(true)
  })
  test('failed more than 15 min ago → hide card', () => {
    // mutant: changed TTL from 15 to 1 min → would hide too early
    const twentyMinAgo = Math.floor(NOW / 1000) - 1200
    expect(shouldShowInProgressCard([failed({ failedAtSecs: twentyMinAgo })], NOW)).toBe(false)
  })
  test('empty list → hide card', () => {
    expect(shouldShowInProgressCard([], NOW)).toBe(false)
  })
  test('failed with no timestamp → show card (conservative)', () => {
    expect(shouldShowInProgressCard([failed({ failedAtSecs: null })], NOW)).toBe(true)
  })
})
