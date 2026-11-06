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
  inProgressCardRows,
  isDownloadRow,
  trayHeaderOverride,
  buildTransferMap,
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
/** "needs attention": file changed on disk while downloading (LD-0105 W2, PR #225 shape). */
function needsAttention(overrides: Partial<ActiveTransferRowLike> = {}): ActiveTransferRowLike {
  return { direction: 'down', done: 50, total: 100, buffered: 0, speedBps: 0, failed: false,
    attention: 'changed_on_this_device', action: 'discard_and_download_again', ...overrides }
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
  test('attention === "changed_on_this_device" → "needs_attention" (priority over failed)', () => {
    // mutant: removed attention check → would return "failed" or "downloading"
    expect(transferUiState(needsAttention())).toBe('needs_attention')
    expect(transferUiState({ ...needsAttention(), failed: true })).toBe('needs_attention')
  })
})

// ── needs_attention state ─────────────────────────────────────────────────────

describe('needs_attention state (Amendment 4, PR #225 IPC shape)', () => {
  test('traySubline returns the same short line as failed (lead ruling 2026-11-06)', () => {
    // Tray is kept short; the full explanation lives in Activity / tooltip.
    // mutant: returned "Discard and download again" → would exceed design length budget
    expect(traySubline(needsAttention())).toBe("Couldn't finish downloading")
  })
  test('traySubline for needs_attention never contains "complete"', () => {
    expect(traySubline(needsAttention()).toLowerCase()).not.toContain('complete')
  })
  test('activityDetail returns the corrected detail sentence (lead ruling 2026-11-06)', () => {
    // The sentence explains what happened and what to do, in one breath.
    // mutant: old copy "The file changed on disk…" → would not match corrected wording
    expect(activityDetail(needsAttention())).toBe(
      "Couldn't finish downloading. The file changed on this device afterwards, so Beebeeb kept it as it is. Discard it to download the file again."
    )
  })
  test('activityDetail for needs_attention ends with a period', () => {
    expect(activityDetail(needsAttention()).endsWith('.')).toBe(true)
  })
  test('progressFraction → null (no bar for needs_attention)', () => {
    expect(progressFraction(needsAttention())).toBeNull()
  })
  test('shouldShowInProgressCard → always true for needs_attention (no TTL)', () => {
    // mutant: treated attention row like failed → would hide after 15 min
    const veryOld = needsAttention({ failedAtSecs: 1 }) // epoch 1 — ancient
    expect(shouldShowInProgressCard([veryOld], Date.now())).toBe(true)
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

// ── trayHeaderOverride ────────────────────────────────────────────────────────

describe('trayHeaderOverride (LD-0105 U1, lead ruling 2026-11-06)', () => {
  test('no transfers → null (normal sync-activity headline shows)', () => {
    // mutant: returned a string for empty → would suppress the headline with no rows
    expect(trayHeaderOverride([])).toBeNull()
  })
  test('all active (not failed, no attention) → null', () => {
    expect(trayHeaderOverride([downloading(), streaming(), buffering()])).toBeNull()
  })
  test("one failed row → singular \"1 file couldn't finish downloading\"", () => {
    // mutant: returned null → would claim "synced" above a visible warn row
    expect(trayHeaderOverride([failed()])).toBe("1 file couldn't finish downloading")
  })
  test("one needs_attention row → same singular override (attention ≠ in-flight)", () => {
    // mutant: treated attention row as active → returned null
    expect(trayHeaderOverride([needsAttention()])).toBe("1 file couldn't finish downloading")
  })
  test("two failed rows → plural \"2 files couldn't finish downloading\"", () => {
    expect(trayHeaderOverride([failed(), failed()])).toBe("2 files couldn't finish downloading")
  })
  test("mixed: one active + one failed → still returns the override (failed row is visible)", () => {
    // mutant: returned null when any row was active → would suppress honest header
    expect(trayHeaderOverride([downloading(), failed()])).toBe("1 file couldn't finish downloading")
  })
  test("mixed failed + needs_attention → both counted", () => {
    expect(trayHeaderOverride([failed(), needsAttention()])).toBe("2 files couldn't finish downloading")
  })
  test("override never contains \"complete\"", () => {
    expect(trayHeaderOverride([failed()])?.toLowerCase()).not.toContain('complete')
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

// ── buildTransferMap ─────────────────────────────────────────────────────────
// P1 fix: keying by name alone collides when two files share the same name in
// different folders (e.g. two "IMG_0001.jpg"). Key on the full relative path.

type WithPath = ActiveTransferRowLike & { name: string; folder: string }

function withPath(name: string, folder: string, overrides: Partial<ActiveTransferRowLike> = {}): WithPath {
  return { direction: 'down', done: 0, total: 100, buffered: 0, speedBps: 0, failed: false, name, folder, ...overrides }
}

describe('buildTransferMap', () => {
  test('two files with the same name but different folders are distinct entries', () => {
    const a = withPath('IMG_0001.jpg', 'Photos/2024')
    const b = withPath('IMG_0001.jpg', 'Photos/2025')
    const map = buildTransferMap([a, b])
    expect(map.size).toBe(2)
    expect(map.get('Photos/2024/IMG_0001.jpg')).toBe(a)
    expect(map.get('Photos/2025/IMG_0001.jpg')).toBe(b)
  })

  test('a root-level file (empty folder) is keyed by name alone', () => {
    const r = withPath('backup.zip', '')
    const map = buildTransferMap([r])
    expect(map.get('backup.zip')).toBe(r)
    expect(map.size).toBe(1)
  })

  test('a nested file key matches its full vault path', () => {
    const r = withPath('report.pdf', 'Documents/2026')
    const map = buildTransferMap([r])
    expect(map.get('Documents/2026/report.pdf')).toBe(r)
  })
})

// ── Uploads stay off the download surfaces (LD-0105 U1 board wiring) ─────────
// The U0 design gives an upload no state on the tray's download line or the "In progress" card
// ("Files arriving on this PC."). On desktop #226 an upload read "Downloading · N% · … waits for the
// whole file" (native rung, e93800c, 13 of 13 samples during a 300 MB upload).

/** The native shape of that rung's upload row, now with the name the Rust side gives it. */
function uploading(overrides: Partial<WithPath> = {}): WithPath {
  return { direction: 'up', done: 33_554_432, total: 295_698_432, buffered: 0, speedBps: 0, failed: false,
    failedAtSecs: null, name: 'r0822-upload-probe.bin', folder: '', ...overrides }
}

describe('uploads are not download rows', () => {
  const NOW = 1_700_000_000_000

  test('transferUiState of an upload is no download state', () => {
    // mutant: `transferUiState` classifies every direction → "downloading"
    expect(transferUiState(uploading())).toBeNull()
    expect(transferUiState(uploading({ failed: true, failedAtSecs: Math.floor(NOW / 1000) }))).toBeNull()
    expect(isDownloadRow(uploading())).toBe(false)
    expect(isDownloadRow(downloading())).toBe(true)
    expect(isDownloadRow(streaming())).toBe(true)
  })

  test('the In progress card never lists an upload as arriving', () => {
    // mutant: `inProgressCardRows` without the `isDownloadRow` filter → the upload is listed
    const down = withPath('holiday-2026.mp4', 'Videos')
    expect(inProgressCardRows([uploading()], NOW)).toEqual([])
    expect(shouldShowInProgressCard([uploading()], NOW)).toBe(false)
    expect(inProgressCardRows([uploading(), down], NOW)).toEqual([down])
  })

  test("the tray's map never gives an upload's row a download line", () => {
    const map = buildTransferMap([uploading({ name: 'notes-draft.txt', folder: 'Documents' })])
    expect(map.get('Documents/notes-draft.txt')).toBeUndefined()
  })

  test('a failed upload never counts as "couldn\'t finish downloading"', () => {
    expect(trayHeaderOverride([uploading({ failed: true, failedAtSecs: 1 })])).toBeNull()
  })
})
