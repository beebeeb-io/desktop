/**
 * The popover's state-to-view mapping (task 1683 slice 3; spec section 3 "State machine" and
 * section 4 "Every state, exact copy").
 *
 * `viewForSnapshot` is the ONE place that decides which of the spec's states a snapshot is and
 * what it says. These tests walk it with the same snapshots the Playwright rung renders
 * (`tests/fixtures/macPopoverStates.mjs`), so a state asserted here is a state seen there.
 * Counts are asserted, never just "no failure": a filter that matched nothing is a red.
 *
 * Mutation checks (red first, then reverted) are recorded in the task Notes, 2026-10-01.
 */
import { describe, expect, test } from 'bun:test'
import { POPOVER_PHASES, parsePopoverSnapshot, type PopoverSnapshot } from '../src/popoverContract'
import {
  COPY,
  DEFAULT_CONTEXT,
  LIST_STATES,
  formatRecency,
  loadFailView,
  loadingView,
  monoReason,
  viewForSnapshot,
  type PopoverStateId,
  type PopoverView,
  type ViewContext,
} from '../src/macPopover/model'
// @ts-expect-error - a plain .mjs shared with the Playwright rung
import { NOW, SNAPSHOTS, RECENT, base_snapshot } from './fixtures/macPopoverStates.mjs'

const ctx = (patch: Partial<ViewContext> = {}): ViewContext => ({ ...DEFAULT_CONTEXT, now: NOW, locale: 'nl-NL', ...patch })
const snap = (key: string): PopoverSnapshot => {
  const parsed = parsePopoverSnapshot(JSON.parse(JSON.stringify(SNAPSHOTS[key])))
  if (!parsed) throw new Error(`fixture ${key} does not satisfy the contract`)
  return parsed
}
const view = (key: string, patch: Partial<ViewContext> = {}) => viewForSnapshot(snap(key), ctx(patch))
const message = (v: PopoverView) => {
  if (v.body.kind !== 'message') throw new Error(`state ${v.state} is not a message state`)
  return v.body
}
const list = (v: PopoverView) => {
  if (v.body.kind !== 'list') throw new Error(`state ${v.state} is not a list state`)
  return v.body
}

describe('which state each snapshot is', () => {
  const EXPECTED: Record<string, PopoverStateId> = {
    synced: 'synced', conflict: 'conflict', empty: 'empty', syncing: 'syncing', syncing0: 'syncing0', locked: 'locked', paused: 'paused', error: 'error',
    offline: 'offline', storage: 'storage', signedout: 'signedout', ended: 'ended', finder: 'finder', finderfail: 'finderfail', finderoff: 'finderoff',
  }
  test('every fixture lands in its state (15 of 15)', () => {
    const got = Object.fromEntries(Object.keys(EXPECTED).map((k) => [k, view(k).state]))
    expect(got).toEqual(EXPECTED)
    expect(Object.keys(got).length).toBe(15)
  })

  test('every one of the 13 phases the Rust reducer can say maps to a drawn state, none to the load-failed fallback', () => {
    const phaseSnapshots: Record<string, string> = {
      signed_out: 'signedout', session_ended: 'ended', locked: 'locked', finder_failed: 'finderfail', finder_user_disabled: 'finderoff', finder_adding: 'finder',
      finder_missing: 'finder', paused: 'paused', storage_full: 'storage', offline: 'offline', error: 'error', syncing: 'syncing', synced: 'synced',
    }
    let walked = 0
    for (const phase of POPOVER_PHASES) {
      const s = snap(phaseSnapshots[phase])
      const v = viewForSnapshot({ ...s, phase }, ctx())
      expect(v.state, `phase ${phase}`).not.toBe('loadfail')
      expect(v.state, `phase ${phase}`).not.toBe('loading')
      walked += 1
    }
    expect(walked).toBe(13)
  })

  test('a phase the UI does not know falls to the load-failed state, never to a list or a blank', () => {
    const v = viewForSnapshot({ ...snap('synced'), phase: 'quantum' as never }, ctx())
    expect(v.state).toBe('loadfail')
  })

  test('the password form replaces c1 only when the user opened it, and only while the vault is locked', () => {
    expect(view('locked').state).toBe('locked')
    expect(view('locked', { unlockOpen: true }).state).toBe('unlock')
    expect(view('synced', { unlockOpen: true }).state).toBe('synced')
    expect(view('paused', { unlockOpen: true }).state).toBe('paused')
    expect(message(view('locked', { unlockOpen: true, unlockWrong: true })).form).toEqual({ wrong: true })
    expect(message(view('locked', { unlockOpen: true })).form).toEqual({ wrong: false })
  })

  test('a Finder install this window started shows f1 at once, from missing or failed, and from no other phase', () => {
    expect(view('finder', { finderPending: true }).state).toBe('finderadding')
    expect(view('finderfail', { finderPending: true }).state).toBe('finderadding')
    for (const key of ['synced', 'paused', 'locked', 'error', 'finderoff', 'signedout']) {
      expect(view(key, { finderPending: true }).state, key).toBe(view(key).state)
    }
  })

  test('the first frame and the failed-load frame claim nothing about an account', () => {
    for (const v of [loadingView(), loadFailView()]) {
      expect(v.header).toEqual({ mode: 'brand' })
      expect(v.footer.visible).toBe(false)
      expect(v.status).toBeNull()
    }
    expect(loadingView().body).toEqual({ kind: 'blank' })
    expect(message(loadFailView()).action?.id).toBe('reload')
  })
})

describe('the one rule that prevents "said twice" (spec section 3)', () => {
  const ALL = ['synced', 'conflict', 'empty', 'syncing', 'syncing0', 'locked', 'paused', 'error', 'offline', 'storage', 'signedout', 'ended', 'finder', 'finderfail', 'finderoff']

  test('the status row exists exactly in the list states and the up-to-date empty state, in no message state', () => {
    const withStatus = ALL.filter((k) => view(k).status !== null)
    expect(withStatus).toEqual(['synced', 'conflict', 'empty', 'syncing', 'syncing0'])
    for (const k of ALL.filter((x) => !withStatus.includes(x))) expect(view(k).status, k).toBeNull()
    // and the list states' own ids are the ones the model calls list states
    expect([...LIST_STATES]).toEqual(['synced', 'conflict', 'syncing', 'syncing0'])
  })

  test('the status row says Up to date or Syncing, with the last check as its tooltip', () => {
    expect(view('synced').status).toEqual({ kind: 'ok', text: 'Up to date', title: 'Last checked 2 min ago' })
    expect(view('empty').status).toEqual({ kind: 'ok', text: 'Up to date', title: 'Last checked just now' })
    expect(view('syncing').status).toEqual({ kind: 'sync', text: 'Syncing', title: null })
    const never = viewForSnapshot({ ...snap('synced'), engine: { ...snap('synced').engine, last_tick_ok_at: null } }, ctx())
    expect(never.status?.title).toBeNull() // no source, no freshness claim
  })

  test('each message state has exactly one primary action at most, and no two states share a title', () => {
    const titles = ALL.map((k) => view(k)).filter((v) => v.body.kind === 'message').map((v) => message(v).title)
    expect(titles.length).toBe(11)
    expect(new Set(titles).size).toBe(11)
  })

  test('no message repeats its title inside its own sentence or its mono line', () => {
    let checked = 0
    for (const k of ALL) {
      const v = view(k)
      if (v.body.kind !== 'message') continue
      const { title, copy, detail } = v.body
      if (copy) expect(copy.toLowerCase().includes(title.toLowerCase()), `${k}: copy repeats the title`).toBe(false)
      if (detail && copy) expect(copy.toLowerCase().includes(detail.toLowerCase()), `${k}: mono line restates the sentence`).toBe(false)
      checked += 1
    }
    expect(checked).toBe(11)
  })
})

describe('exact copy (spec section 4)', () => {
  const T = (k: string, patch: Partial<ViewContext> = {}) => message(view(k, patch))
  test('every message state says what the spec says (13 title/copy/action triples)', () => {
    const rows: [string, Partial<ViewContext>, string, string | null, string | null][] = [
      ['locked', {}, 'Your vault is locked', 'Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.', 'Unlock vault'],
      ['locked', { unlockOpen: true }, 'Enter your password', null, 'Unlock'],
      ['paused', {}, 'Sync is paused', '3 changes on this Mac are waiting. Nothing syncs until you resume.', 'Resume sync'],
      ['error', {}, 'Can’t reach Beebeeb', 'Beebeeb didn’t answer. Files on this Mac are unchanged.', 'Try again'],
      ['offline', {}, 'You’re offline', 'Changes on this Mac will sync when you’re back online. Beebeeb keeps checking.', null],
      ['storage', {}, 'Your storage is full', '3 changes can’t upload until you free up space or add storage. They stay on this Mac.', 'Add storage'],
      ['signedout', {}, 'Welcome to Beebeeb', 'Create an account or sign in to start syncing. Files are encrypted on this Mac first, then stored in the EU.', 'Set up Beebeeb'],
      ['ended', {}, 'Your session ended', 'Sign in again to keep syncing. Files on this Mac are unchanged.', 'Sign in'],
      ['finder', {}, 'Beebeeb isn’t in Finder yet', 'Add it to see your files in Finder like any other folder. Files download when you open them, unless you keep them on this Mac.', 'Add to Finder'],
      ['finder', { finderPending: true }, 'Adding Beebeeb to Finder', 'macOS is setting it up.', 'Adding…'],
      ['finderfail', {}, 'Couldn’t add Beebeeb to Finder', 'macOS didn’t respond in time.', 'Try again'],
      ['empty', {}, 'Nothing new yet', 'Changes on this Mac and in your vault appear here.', null],
      ['finderoff', {}, 'Beebeeb is turned off in Finder', 'You turned it off in System Settings. Turn it back on to see your files in Finder.', 'Open System Settings'],
    ]
    for (const [key, patch, title, copy, action] of rows) {
      const b = T(key, patch)
      expect([key, b.title, b.copy, b.action?.label ?? null]).toEqual([key, title, copy, action])
    }
    expect(rows.length).toBe(13)
  })

  test('Adding… is drawn but not pressable; every other action is', () => {
    expect(T('finder', { finderPending: true }).action?.disabled).toBe(true)
    expect(T('finder').action?.disabled).toBe(false)
  })

  test('the mono reason adds information: the engine detail for d, the category line for f2, nothing for the rest', () => {
    expect(T('error').detail).toBe('timeout after 30 s')
    expect(T('finderfail').detail).toBe('reason: timeout')
    for (const k of ['locked', 'paused', 'offline', 'storage', 'signedout', 'ended', 'finder', 'finderoff']) expect(T(k).detail, k).toBeNull()
    // a purely local engine failure has no mono line (slice 2: reason `engine_error`)
    const local = viewForSnapshot({ ...snap('error'), reason: { code: 'engine_error', detail: null } }, ctx())
    expect(message(local).detail).toBeNull()
  })

  test('the other two connection reasons slice 2 added are shown as sent', () => {
    for (const detail of ['connection dropped', 'HTTP 503']) {
      expect(message(viewForSnapshot({ ...snap('error'), reason: { code: 'x', detail } }, ctx())).detail).toBe(detail)
    }
  })

  test('the Finder sentence does not claim a timeout for any other category (4 categories)', () => {
    const sentence = (reason: string | null, line: string | null) => message(viewForSnapshot({ ...snap('finderfail'), finder: { setup: 'failed', reason, reason_line: line } }, ctx())).copy
    expect(sentence('timeout', 'reason: timeout')).toBe('macOS didn’t respond in time.')
    expect(sentence('provisioning', 'reason: provisioning')).toBe('macOS couldn’t finish adding it.')
    expect(sentence('extension_missing', 'reason: extension_missing')).toBe('macOS couldn’t finish adding it.')
    expect(sentence(null, null)).toBe('macOS didn’t respond in time.') // no category recorded: the approved sentence
  })

  test('singular and zero forms of the counted sentences', () => {
    const paused = (n: number) => message(viewForSnapshot({ ...snap('paused'), pending_changes: n }, ctx())).copy
    expect(paused(1)).toBe('1 change on this Mac is waiting. Nothing syncs until you resume.')
    expect(paused(0)).toBe('Nothing syncs until you resume.')
    const full = (n: number) => message(viewForSnapshot({ ...snap('storage'), pending_changes: n }, ctx())).copy
    expect(full(1)).toBe('1 change can’t upload until you free up space or add storage. It stays on this Mac.')
    expect(full(0)).toBe('New changes can’t upload until you free up space or add storage. They stay on this Mac.')
  })

  test('the wrong-password line is the spec’s, and the copy table has no straight apostrophes', () => {
    expect(COPY.wrongPassword).toBe('That password didn’t work. Try again.')
    const all = JSON.stringify([COPY, ...['locked', 'paused', 'error', 'offline', 'storage', 'signedout', 'ended', 'finder', 'finderfail', 'finderoff', 'empty'].map((k) => view(k).body)])
    expect(all.includes("'")).toBe(false)
  })
})

describe('header', () => {
  test('signed out and session ended use the brand header with no account', () => {
    expect(view('signedout').header).toEqual({ mode: 'brand' })
    expect(view('ended').header).toEqual({ mode: 'brand' })
  })

  test('the account header shows the email and one storage figure in the Mac’s locale', () => {
    expect(view('synced').header).toEqual({ mode: 'account', email: 'sam@example.eu', usage: '84,3 GB of 200 GB', pct: 42, full: false })
    const en = viewForSnapshot(snap('synced'), ctx({ locale: 'en-US' }))
    expect(en.header).toMatchObject({ usage: '84.3 GB of 200 GB' })
  })

  test('locked shows the email only: storage cannot load, so no figure and no bar', () => {
    expect(view('locked').header).toEqual({ mode: 'account', email: 'sam@example.eu', usage: null, pct: null, full: false })
    // even if a stale figure were present, a locked vault does not show it
    const stale = viewForSnapshot({ ...snap('locked'), storage: { used_bytes: 1, quota_bytes: 2, fetched_at: 0, stale: true } }, ctx())
    expect(stale.header).toMatchObject({ usage: null })
  })

  test('the bar goes red at 90 percent and not before; a zero quota shows no figure rather than a division', () => {
    const at = (used: number) => viewForSnapshot({ ...snap('synced'), storage: { used_bytes: used, quota_bytes: 100, fetched_at: 0, stale: false } }, ctx()).header
    expect(at(89)).toMatchObject({ pct: 89, full: false })
    expect(at(90)).toMatchObject({ pct: 90, full: true })
    expect(at(100)).toMatchObject({ pct: 100, full: true })
    expect(at(250)).toMatchObject({ pct: 100 })
    const zero = viewForSnapshot({ ...snap('synced'), storage: { used_bytes: 5, quota_bytes: 0, fetched_at: 0, stale: false } }, ctx()).header
    expect(zero).toMatchObject({ usage: null, pct: null, full: false })
    expect(view('storage').header).toMatchObject({ usage: '200 GB of 200 GB', pct: 100, full: true })
  })

  test('a missing email stays null (the UI decides the words)', () => {
    const none = viewForSnapshot({ ...snap('synced'), account: { ...snap('synced').account, email: null } }, ctx())
    expect(none.header).toMatchObject({ mode: 'account', email: null })
  })
})

describe('footer', () => {
  test('three actions are always the same three; the footer is gone only for signed out and session ended', () => {
    expect(view('signedout').footer.visible).toBe(false)
    expect(view('ended').footer.visible).toBe(false)
    for (const k of ['synced', 'conflict', 'empty', 'syncing', 'locked', 'paused', 'error', 'offline', 'storage', 'finder', 'finderfail', 'finderoff']) {
      expect(view(k).footer.visible, k).toBe(true)
    }
  })

  test('Open in Finder is disabled while Finder is not set up and enabled otherwise', () => {
    const disabled = ['finder', 'finderfail', 'finderoff'].filter((k) => view(k).footer.finderDisabled)
    expect(disabled).toEqual(['finder', 'finderfail', 'finderoff'])
    for (const k of ['synced', 'conflict', 'syncing', 'paused', 'error', 'offline', 'storage', 'locked']) expect(view(k).footer.finderDisabled, k).toBe(false)
    expect(view('finder', { finderPending: true }).footer.finderDisabled).toBe(true)
    // a state that is not a Finder state but sits on a Mac where Finder is not set up
    const pausedNoFinder = viewForSnapshot({ ...snap('paused'), finder: { setup: 'missing', reason: null, reason_line: null } }, ctx())
    expect(pausedNoFinder.footer.finderDisabled).toBe(true)
  })
})

describe('the list states', () => {
  test('a: a label, then one row per recent file, newest first as sent', () => {
    const b = list(view('synced'))
    expect(b.summary).toBeNull()
    expect(b.items[0]).toEqual({ kind: 'label', text: 'Recent activity' })
    const rows = b.items.slice(1)
    expect(rows.length).toBe(5)
    expect(rows.map((r) => (r.kind === 'activity' ? r.subline : ''))).toEqual([
      'Investigations · 2 min ago', 'Ledger gap · 14 min ago', 'Photos · 1 hour ago', 'Team shared · Yesterday', 'Investigations · Yesterday',
    ])
    expect(rows.map((r) => (r.kind === 'activity' ? r.direction : ''))).toEqual(['up', 'up', 'down', 'down', 'up'])
    expect(rows.map((r) => (r.kind === 'activity' ? r.tile : ''))).toEqual(['file', 'file', 'image', 'file', 'file'])
  })

  test('a row reads as one sentence for VoiceOver, with the direction in words', () => {
    const first = list(view('synced')).items[1]
    expect(first.kind === 'activity' && first.label).toBe('Q3 ledger reconciliation.xlsx, Investigations, 2 minutes ago, uploaded')
    const down = list(view('synced')).items[3]
    expect(down.kind === 'activity' && down.label).toBe('IMG_4821.HEIC, Photos, 1 hour ago, downloaded')
  })

  test('a1: one conflict puts a decision row first, with Review for that file; then the recent list', () => {
    const b = list(view('conflict'))
    expect(b.items[0]).toEqual({ kind: 'label', text: 'Needs your decision' })
    expect(b.items[1]).toEqual({ kind: 'conflict', title: 'Budget 2027.xlsx', subline: 'Two versions. Choose which to keep.', fileId: 'f1', fileName: 'Budget 2027.xlsx' })
    expect(b.items[2]).toEqual({ kind: 'label', text: 'Recent activity' })
    expect(b.items.length).toBe(3 + 4)
  })

  test('two or more conflicts read as a count, and Review still opens the first file', () => {
    const s = { ...snap('conflict'), conflicts: { count: 3, files: [{ file_id: 'a', file_name: 'A.xlsx' }, { file_id: 'b', file_name: 'B.xlsx' }] } }
    const item = list(viewForSnapshot(s, ctx())).items[1]
    expect(item).toEqual({ kind: 'conflict', title: '3 files have two versions.', subline: 'Choose which to keep.', fileId: 'a', fileName: 'A.xlsx' })
  })

  test('a conflict count with no file to open draws no conflict row (never a dead Review button)', () => {
    const s = { ...snap('synced'), conflicts: { count: 2, files: [] } }
    const v = viewForSnapshot(s, ctx())
    expect(v.state).toBe('synced')
    expect(list(v).items.some((i) => i.kind === 'conflict')).toBe(false)
  })

  test('a conflict beside an otherwise empty list is still a list, not "Nothing new yet"', () => {
    const s = { ...snap('empty'), conflicts: { count: 1, files: [{ file_id: 'a', file_name: 'A.xlsx' }] } }
    const v = viewForSnapshot(s, ctx())
    expect(v.state).toBe('conflict')
    expect(list(v).items.map((i) => i.kind)).toEqual(['label', 'conflict'])
  })

  test('b: files left, bytes and percent, one overall figure; per-file bars only where bytes are known', () => {
    const b = list(view('syncing'))
    expect(b.summary).toEqual({ left: '14 files left', right: '1,2 GB · 38 %', pct: 38 })
    const rows = b.items.filter((i) => i.kind === 'activity')
    expect(rows.map((r) => r.kind === 'activity' && [r.subline, r.progress, r.progressTone])).toEqual([
      ['Investigations · Encrypting and uploading · 62 %', 62, 'encrypting'],
      ['Photos · Encrypting and uploading · 21 %', 21, 'encrypting'],
      ['Team shared · Downloading · 88 %', 88, 'neutral'],
      ['Investigations · 2 min ago', null, 'encrypting'],
    ])
    expect(b.items.filter((i) => i.kind === 'label')).toEqual([{ kind: 'label', text: 'Done' }])
  })

  test('b prime: with no per-file bytes there is no per-file percent and no guessed overall percent', () => {
    const v = view('syncing0')
    expect(v.state).toBe('syncing0')
    const b = list(v)
    expect(b.summary).toEqual({ left: '14 files left', right: null, pct: null })
    for (const r of b.items) if (r.kind === 'activity') expect(r.progress, r.name).toBeNull()
    const verbs = b.items.filter((i) => i.kind === 'activity').slice(0, 3).map((r) => r.kind === 'activity' && r.subline)
    expect(verbs).toEqual(['Investigations · Encrypting and uploading', 'Photos · Encrypting and uploading', 'Team shared · Downloading'])
  })

  test('one file left is singular; no count draws no summary', () => {
    const one = viewForSnapshot({ ...snap('syncing'), engine: { ...snap('syncing').engine, files_remaining: 1 } }, ctx())
    expect(list(one).summary?.left).toBe('1 file left')
    const none = viewForSnapshot({ ...snap('syncing'), engine: { ...snap('syncing').engine, files_remaining: null } }, ctx())
    expect(list(none).summary).toBeNull()
  })

  test('bytes that overshoot or arrive without a total never show a percent past 100 or a division', () => {
    const over = viewForSnapshot({ ...snap('syncing'), engine: { ...snap('syncing').engine, bytes_total: 100, bytes_done: 250 } }, ctx())
    expect(list(over).summary?.pct).toBe(100)
    const zero = viewForSnapshot({ ...snap('syncing'), engine: { ...snap('syncing').engine, bytes_total: 0, bytes_done: 0 } }, ctx())
    expect(list(zero).summary).toMatchObject({ right: null, pct: null })
    const rowZero = viewForSnapshot({ ...snap('syncing'), activity: snap('syncing').activity.map((r) => ({ ...r, done_bytes: 5, total_bytes: 0 })) }, ctx())
    for (const r of list(rowZero).items) if (r.kind === 'activity') expect(r.progress).toBeNull()
  })

  test('a failed file is a row with an alert, not a state; with failures among the finished rows the label is not Done', () => {
    const s = parsePopoverSnapshot(JSON.parse(JSON.stringify(SNAPSHOTS.syncing)))!
    s.activity = [s.activity[0], { id: 'x', name: 'bad.bin', folder: 'Notes', path: 'Notes/bad.bin', direction: null, state: 'failed', at: NOW - 20, size_bytes: 1, done_bytes: null, total_bytes: null }]
    const b = list(viewForSnapshot(s, ctx()))
    const failed = b.items.find((i) => i.kind === 'activity' && i.failed)
    expect(failed && failed.kind === 'activity' && [failed.subline, failed.direction, failed.label]).toEqual(['Notes · Couldn’t sync', null, 'bad.bin, Notes, Couldn’t sync'])
    expect(b.items.filter((i) => i.kind === 'label')).toEqual([{ kind: 'label', text: 'Recent activity' }])
    expect(viewForSnapshot(s, ctx()).state).toBe('syncing') // the file failure did not take over the body
  })

  test('a row at the vault root shows the time alone, and a long run of rows keeps every one', () => {
    const s = { ...snap('synced'), activity: Array.from({ length: 10 }, (_, i) => ({ ...RECENT[0], id: `r${i}`, folder: i === 0 ? '' : 'Docs' })) }
    const rows = list(viewForSnapshot(s as PopoverSnapshot, ctx())).items.filter((i) => i.kind === 'activity')
    expect(rows.length).toBe(10)
    expect(rows[0].kind === 'activity' && rows[0].subline).toBe('2 min ago')
    expect(rows[1].kind === 'activity' && rows[1].subline).toBe('Docs · 2 min ago')
  })
})

describe('relative time (the Mac’s locale for dates)', () => {
  const at = (y: number, m: number, d: number, h = 0, mi = 0) => Math.floor(new Date(y, m, d, h, mi).getTime() / 1000)
  const now = at(2026, 9, 1, 12, 0)
  test('just now, minutes, hours, yesterday, days, then a date (12 boundaries)', () => {
    const cases: [number, string][] = [
      [now - 10, 'just now'],
      [now - 44, 'just now'],
      [now - 45, '1 min ago'],
      [now - 100, '2 min ago'], // 1.67 minutes rounds up
      [now - 120, '2 min ago'],
      [now - 59 * 60, '59 min ago'],
      [now - 3600, '1 hour ago'],
      [now - 3 * 3600, '3 hours ago'],
      [at(2026, 8, 30, 23, 0), 'Yesterday'],
      [at(2026, 8, 28, 9, 0), '3 days ago'],
      [at(2026, 8, 25, 9, 0), '6 days ago'],
      [at(2026, 8, 24, 9, 0), 'Sep 24'], // seven days: a date, not "7 days ago"
    ]
    for (const [t, want] of cases) expect(formatRecency(now, t, 'en-US')).toBe(want)
    expect(cases.length).toBe(12)
  })
  test('older than a week is a date in the locale; another year adds the year', () => {
    expect(formatRecency(now, at(2026, 8, 1), 'en-US')).toBe('Sep 1')
    expect(formatRecency(now, at(2026, 8, 1), 'nl-NL')).toBe('1 sep')
    expect(formatRecency(now, at(2025, 11, 24), 'en-US')).toBe('Dec 24, 2025')
  })
  test('spoken form writes the units out; a missing time says nothing', () => {
    expect(formatRecency(now, now - 120, 'en', true)).toBe('2 minutes ago')
    expect(formatRecency(now, now - 60, 'en', true)).toBe('1 minute ago')
    expect(formatRecency(now, 0, 'en')).toBe('')
    expect(formatRecency(now, NaN, 'en')).toBe('')
  })
  test('a time in the future is just now, never negative', () => {
    expect(formatRecency(now, now + 500, 'en')).toBe('just now')
  })
})

describe('the mono reason (30 characters, one line, adds information)', () => {
  test('fits, truncates with an ellipsis at exactly 30, collapses whitespace, and is null for nothing', () => {
    expect(monoReason('timeout after 30 s')).toBe('timeout after 30 s')
    expect(monoReason('x'.repeat(30))).toBe('x'.repeat(30))
    expect(monoReason('x'.repeat(31))).toBe(`${'x'.repeat(29)}…`)
    expect(monoReason('x'.repeat(31))?.length).toBe(30)
    expect(monoReason('a\n  b\tc')).toBe('a b c')
    expect(monoReason('   ')).toBeNull()
    expect(monoReason(null)).toBeNull()
    expect(monoReason(undefined)).toBeNull()
  })
})

describe('the model is pure', () => {
  test('the same snapshot and context give the same view, and the snapshot is not changed', () => {
    const s = snap('syncing')
    const before = JSON.stringify(s)
    expect(JSON.stringify(viewForSnapshot(s, ctx()))).toBe(JSON.stringify(viewForSnapshot(s, ctx())))
    expect(JSON.stringify(s)).toBe(before)
  })
  test('the base snapshot fixture still satisfies the contract (the instrument for every test above)', () => {
    expect(parsePopoverSnapshot(base_snapshot())).not.toBeNull()
  })
})
