/**
 * Home's three additions, as pure decisions (task 1683 slice 4, the Home add-on; spec
 * `docs/specs/2026-10-24-macos-two-surface-app.md` §7.6 Home, §10.2 rows 3, 34 and 45): the counts line, which Review
 * entries "Needs your attention" shows, and the "Recent activity" rows. Nothing here renders; tests/homeSections.test.tsx
 * runs the sections and tests/macApp.test.tsx the pane.
 */
import { describe, expect, test } from 'bun:test'
import type { FileOverview } from '../src/desktopApi'
import {
  HOME_ACTIVITY_ROWS,
  HOME_ATTENTION_LIMIT,
  activityRowsForHome,
  attentionForHome,
  homeCounts,
  relativeAgo,
} from '../src/homeModel'
import type { ActivityRow } from '../src/popoverContract'

const overview = (over: Partial<FileOverview> = {}): FileOverview => ({
  by_status: [
    { status: 'cloud_only', count: 8311, bytes: 9 },
    { status: 'local', count: 1204, bytes: 9 },
    { status: 'error', count: 2, bytes: 1 },
  ],
  recent: [], total_files: 9517, total_bytes: 19, pinned_count: 40, pinned_bytes: 4, pinned_folders: 3,
  ...over,
})

describe('the counts line (§7.6: "On this Mac {n} files · Online-only {n} · Kept on this Mac {n} folders")', () => {
  test('three parts, a label and a value each, in the hi-fi\'s order, numbers in the Mac\'s grouping', () => {
    expect(homeCounts(overview(), 'en')).toEqual([
      { label: 'On this Mac', value: '1,204 files' },
      { label: 'Online-only', value: '8,311' },
      { label: 'Kept on this Mac', value: '3 folders' },
    ])
    expect(homeCounts(overview(), 'nl')?.[0].value).toBe('1.204 files')
  })

  test('"On this Mac" is the files whose bytes are here (the local bucket) and "Online-only" the cloud-only bucket, as the Windows shell derives them', () => {
    const parts = homeCounts(overview({ by_status: [{ status: 'local', count: 5, bytes: 1 }, { status: 'cloud_only', count: 7, bytes: 1 }, { status: 'uploading', count: 99, bytes: 1 }], total_files: 111 }), 'en')!
    expect(parts[0].value).toBe('5 files')
    expect(parts[1].value).toBe('7')
  })

  test('one is singular; a status with no files is zero, not missing', () => {
    expect(homeCounts(overview({ by_status: [{ status: 'local', count: 1, bytes: 1 }], total_files: 1, pinned_folders: 1 }), 'en')).toEqual([
      { label: 'On this Mac', value: '1 file' },
      { label: 'Online-only', value: '0' },
      { label: 'Kept on this Mac', value: '1 folder' },
    ])
  })

  test('no files mirrored (no sync root, no database, an empty vault) is no line at all, never "0 files"', () => {
    expect(homeCounts(overview({ by_status: [], total_files: 0, pinned_folders: 0 }), 'en')).toBeNull()
    expect(homeCounts(null, 'en')).toBeNull()
  })

  test('a build whose overview carries no folder count leaves that part out: it cannot say "0 folders" by accident', () => {
    const parts = homeCounts(overview({ pinned_folders: undefined }), 'en')!
    expect(parts.map((p) => p.label)).toEqual(['On this Mac', 'Online-only'])
    expect(homeCounts(overview({ pinned_folders: 0 }), 'en')!.at(-1)).toEqual({ label: 'Kept on this Mac', value: '0 folders' })
  })

  test('nothing says PC', () => {
    expect(JSON.stringify(homeCounts(overview(), 'en'))).not.toMatch(/\bPC\b/)
  })
})

const entry = (id: string) => ({ id, file_id: `f-${id}`, file_name: `${id}.md`, kind: 'conflict', status: 's', updated_at: 1, detail: 'd', action: 'open_conflict' })
const ready = (n: number) => ({ status: 'ready' as const, entries: Array.from({ length: n }, (_, i) => entry(`e${i}`)), reason: null })

describe('"Needs your attention" shows the top three Review entries (§7.6)', () => {
  test('three of five, in the feed\'s order, and how many more wait in Review', () => {
    const got = attentionForHome(ready(5))
    expect(HOME_ATTENTION_LIMIT).toBe(3)
    expect(got.entries.map((e) => e.id)).toEqual(['e0', 'e1', 'e2'])
    expect(got.more).toBe(2)
  })

  test('fewer than three: all of them, none more', () => {
    expect(attentionForHome(ready(2))).toMatchObject({ more: 0 })
    expect(attentionForHome(ready(2)).entries).toHaveLength(2)
    expect(attentionForHome(ready(3)).more).toBe(0)
  })

  test('a feed that is loading or failed shows none: Home never guesses', () => {
    expect(attentionForHome({ status: 'loading', entries: [], reason: null })).toEqual({ entries: [], more: 0 })
    expect(attentionForHome({ status: 'failed', entries: [], reason: 'x' })).toEqual({ entries: [], more: 0 })
  })
})

const now = new Date(2026, 9, 25, 14, 30)
const secs = (y: number, mo: number, d: number, h: number, mi: number) => Math.floor(new Date(y, mo, d, h, mi).getTime() / 1000)
const row = (over: Partial<ActivityRow> = {}): ActivityRow => ({
  id: 'a1', name: 'Q3 ledger reconciliation.xlsx', folder: 'Investigations', path: 'Investigations/Q3 ledger reconciliation.xlsx',
  direction: 'up', state: 'done', at: secs(2026, 9, 25, 14, 28), size_bytes: 48_000, done_bytes: 48_000, total_bytes: 48_000, ...over,
})

describe('"Recent activity" is the popover\'s rows, more of them (§7.6)', () => {
  test('the window asks for eight', () => {
    expect(HOME_ACTIVITY_ROWS).toBe(8)
  })

  test('an upload and a download that finished: the word, the folder, how long ago', () => {
    const [up, down] = activityRowsForHome([row(), row({ id: 'a2', name: 'Plan.pdf', folder: 'Board', direction: 'down', at: secs(2026, 9, 25, 11, 30) })], now)
    expect(up).toMatchObject({ id: 'a1', name: 'Q3 ledger reconciliation.xlsx', icon: 'up', line: 'Uploaded · Investigations · 2 minutes ago', tone: 'neutral' })
    expect(down).toMatchObject({ icon: 'down', line: 'Downloaded · Board · 3 hours ago' })
  })

  test('a transfer under way says so, with its percentage only when both byte counts are known', () => {
    const [known, unknown] = activityRowsForHome([
      row({ state: 'active', done_bytes: 62, total_bytes: 100 }),
      row({ id: 'a2', state: 'active', direction: 'down', done_bytes: null, total_bytes: null }),
    ], now)
    expect(known.line).toBe('Uploading · 62 % · Investigations · 2 minutes ago')
    expect(unknown.line).toBe('Downloading · Investigations · 2 minutes ago')
    expect(known.tone).toBe('busy')
  })

  test('a percentage never leaves 0 to 100, and a zero total is no percentage', () => {
    const [over, none] = activityRowsForHome([row({ state: 'active', done_bytes: 300, total_bytes: 100 }), row({ id: 'a2', state: 'active', done_bytes: 0, total_bytes: 0 })], now)
    expect(over.line.startsWith('Uploading · 100 %')).toBe(true)
    expect(none.line.startsWith('Uploading · Investigations')).toBe(true)
  })

  test('a failed row says it could not sync, in the alert tone, with no direction', () => {
    const [failed] = activityRowsForHome([row({ state: 'failed', direction: null })], now)
    expect(failed).toMatchObject({ icon: 'alert', line: 'Couldn’t sync · Investigations · 2 minutes ago', tone: 'alert' })
  })

  test('a file at the root has no folder part, and the whole sentence reads aloud as the popover\'s does', () => {
    const [r] = activityRowsForHome([row({ folder: '' })], now)
    expect(r.line).toBe('Uploaded · 2 minutes ago')
    expect(r.label).toBe('Q3 ledger reconciliation.xlsx, uploaded, 2 minutes ago')
    expect(activityRowsForHome([row()], now)[0].label).toBe('Q3 ledger reconciliation.xlsx, uploaded, Investigations, 2 minutes ago')
  })

  test('the path rides along for a tooltip', () => {
    expect(activityRowsForHome([row()], now)[0].path).toBe('Investigations/Q3 ledger reconciliation.xlsx')
  })
})

describe('how long ago', () => {
  const at = (minutesAgo: number) => Math.floor(now.getTime() / 1000) - minutesAgo * 60
  test('just now, minutes, hours, yesterday, then the day', () => {
    expect(relativeAgo(at(0), now)).toBe('just now')
    expect(relativeAgo(at(0.5), now)).toBe('just now')
    expect(relativeAgo(at(1), now)).toBe('1 minute ago')
    expect(relativeAgo(at(2), now)).toBe('2 minutes ago')
    expect(relativeAgo(at(59), now)).toBe('59 minutes ago')
    expect(relativeAgo(at(60), now)).toBe('1 hour ago')
    expect(relativeAgo(at(61 * 3), now)).toBe('3 hours ago')
    expect(relativeAgo(secs(2026, 9, 24, 9, 0), now)).toBe('yesterday')
    expect(relativeAgo(secs(2026, 9, 12, 9, 0), now)).toBe('12 Oct')
    expect(relativeAgo(secs(2025, 9, 12, 9, 0), now)).toBe('12 Oct 2025')
  })
  test('a clock that is ahead is just now, and a time nobody can read is no text', () => {
    expect(relativeAgo(Math.floor(now.getTime() / 1000) + 600, now)).toBe('just now')
    expect(relativeAgo(0, now)).toBeNull()
    expect(relativeAgo(Number.NaN, now)).toBeNull()
  })
})
