/**
 * The Review pane (task 1683 slice 4; spec §7.6, §10.2 rows 11, 13, 16, 17; hi-fi section 3): the real `ReviewPane`
 * declaration, with the two sheets it opens stubbed to props recorders (their own tests are
 * tests/conflictSheet.test.tsx and tests/versionHistorySheet.test.tsx).
 * What this proves: loading, failed, empty and full states in the spec's words; one row per feed entry with the new
 * tag, name, line and at most one action; each action does what its label says; a resolved conflict asks the window
 * to read the feed again; Version history is a sheet over Review and closes where it was opened.
 * What it does NOT prove: layout, or that the feed is right (tests/reviewFeed.test.ts, src-tauri).
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import * as desktopApi from '../src/desktopApi'
import * as parts from '../src/macSettingsParts'
import { makeReviewActions } from '../src/reviewActions'
import * as reviewModel from '../src/reviewModel'
import { loadComponent, mount, type Mounted } from './fixtures/componentHarness'
import { button, buttons, buttonLabel, find, lines, press, settle } from './fixtures/sheetHarness'

const ReviewEntryRow = loadComponent('ReviewRow.tsx', 'ReviewEntryRow', { React: { createElement }, Btn: parts.Btn, ...reviewModel })
const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const now = new Date()
const today = (h: number, mi: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate(), h, mi).getTime() / 1000

const conflict: desktopApi.VersionConflictEntry = {
  id: 'conflict:f1', file_id: 'f1', file_name: 'Board notes, 29 Sept.md', folder: 'Team shared', kind: 'conflict', status: 'needs review',
  updated_at: today(0, 0), detail: 'Changed on two devices.', action: 'open_conflict',
}
const upload: desktopApi.VersionConflictEntry = {
  id: 'op:2', file_id: 'f2', file_name: 'Site visit, north wing.mov', folder: 'Photos', kind: 'failed_upload', status: 'upload review',
  updated_at: today(0, 1), detail: 'Couldn’t upload, will retry', action: 'review_upload',
}
const quota: desktopApi.VersionConflictEntry = { ...upload, id: 'op:3', file_id: 'f3', file_name: 'Archive.zip', kind: 'quota_failure', detail: 'This change can’t upload until you free up space or add storage. It stays on this Mac.' }
const signedOut: desktopApi.VersionConflictEntry = { ...upload, id: 'op:4', file_id: 'f4', file_name: 'Plan.pdf', kind: 'auth_failure', action: 'sign_in_again', detail: 'Your session has expired. Sign in again to resume syncing.' }

type Feed = { status: 'loading' | 'ready' | 'failed'; entries: desktopApi.VersionConflictEntry[]; reason: string | null }
const ready = (...entries: desktopApi.VersionConflictEntry[]): Feed => ({ status: 'ready', entries, reason: null })

function open(feed: Feed, extra: { historyOpen?: boolean; bindings?: Record<string, unknown> } = {}) {
  const sheets: Array<{ kind: string; props: any }> = []
  const events: string[] = []
  const urls: string[] = []
  const m = mount('ReviewPane.tsx', 'ReviewPane', {
    expand: true,
    backend: {} as any,
    props: { feed, historyOpen: extra.historyOpen ?? false, onHistoryClose: () => events.push('history-closed'), onChanged: () => events.push('changed') },
    bindings: {
      ...desktopApi,
      ...reviewModel,
      Btn: parts.Btn, Note: parts.Note, SettingsIcon: parts.SettingsIcon,
      makeReviewActions, ReviewEntryRow,
      ConflictSheet: (props: any) => { sheets.push({ kind: 'conflict', props }); return createElement('div', { 'data-sheet': 'conflict', 'data-file': props.entry.file_id }) },
      VersionHistorySheet: (props: any) => { sheets.push({ kind: 'history', props }); return createElement('div', { 'data-sheet': 'history' }) },
      openUrl: async (url: string) => { urls.push(url); return { ok: true, value: undefined } },
      ...extra.bindings,
    },
  })
  mounted.push(m)
  return { m, sheets, events, urls }
}
/** The sheet stand-ins only run when the tree is expanded: read the tree, then the latest props they were given. */
const latest = (m: Mounted, sheets: Array<{ kind: string; props: any }>) => { m.elements(); return sheets[sheets.length - 1] }
const sheetsShown = (m: Mounted) => find(m, (el) => el.props['data-sheet'] !== undefined).map((el) => el.props['data-sheet'])

describe('states', () => {
  test('loading says so and shows nothing else', () => {
    const { m } = open({ status: 'loading', entries: [], reason: null })
    expect(lines(m)).toEqual(['Loading…'])
  })

  test('a feed that cannot be read is one alert with the reason and a way to try again that asks the window to read again', async () => {
    const { m, events } = open({ status: 'failed', entries: [], reason: 'state.db is locked' })
    const alerts = find(m, (el) => el.props.role === 'alert')
    expect(alerts).toHaveLength(1)
    expect(lines(m)).toEqual(expect.arrayContaining(['Couldn’t load Review', 'state.db is locked']))
    await press(m, 'Try again')
    expect(events).toEqual(['changed'])
  })

  test('nothing to review: the intro, then "Nothing needs your attention."', () => {
    const { m } = open(ready())
    expect(lines(m)).toEqual(['Files that need a decision from you, and changes Beebeeb couldn’t send yet.', 'Nothing needs your attention.'])
    expect(buttons(m)).toHaveLength(0)
  })
})

describe('rows (§7.6)', () => {
  test('one row per entry, in the feed\'s order: tag, name, when and detail, and the action', () => {
    const { m } = open(ready(conflict, upload))
    const rows = find(m, (el) => el.props['data-row'] === 'review')
    expect(rows).toHaveLength(2)
    const flat = (row: any) => lines({ ...m, tree: () => row } as Mounted)
    expect(flat(rows[0])).toEqual(['Conflict', 'Board notes, 29 Sept.md', 'Team shared · today 00:00 · Changed on two devices.', 'Decide…'])
    expect(flat(rows[1])).toEqual(['Upload', 'Site visit, north wing.mov', 'Photos · today 00:01 · Couldn’t upload, will retry'])
  })

  test('the renamed tags show, and the retired ones do not', () => {
    const stale = { ...upload, id: 'op:5', kind: 'stale_base' as const }
    const meta = { ...upload, id: 'op:6', kind: 'metadata' as const }
    const perm = { ...upload, id: 'op:7', kind: 'permission_failure' as const }
    const { m } = open(ready(stale, meta, perm, quota))
    const text = lines(m).join(' | ')
    for (const tag of ['Changed elsewhere', 'Rename or move', 'No permission', 'Storage full']) expect(text).toContain(tag)
    for (const old of ['Stale base', 'Metadata', 'Quota', 'Permission']) expect(text).not.toContain(old)
  })

  test('only a conflict, a storage failure and a sign-in failure have a button, labelled as the spec says', () => {
    const { m } = open(ready(conflict, upload, quota, signedOut))
    expect(buttons(m).map(buttonLabel)).toEqual(['Decide…', 'Add storage', 'Sign in again'])
  })

  test('no row button is the amber primary: Review has no single primary action, and amber is for that', () => {
    const { m } = open(ready(conflict, quota, signedOut))
    expect(buttons(m).filter((el) => String(el.props.className).includes('ms-btn--primary'))).toHaveLength(0)
  })

  test('the file name wraps and carries user text only as text, never as markup', () => {
    const nasty = { ...conflict, file_name: '<img src=x onerror=alert(1)>.md' }
    const { m } = open(ready(nasty))
    expect(lines(m)).toContain('<img src=x onerror=alert(1)>.md')
  })
})

describe('what each action does', () => {
  test('"Decide…" opens the conflict sheet for THAT file, over Review, and the sheet closing returns to the list', async () => {
    const { m, sheets } = open(ready(conflict, upload))
    expect(sheetsShown(m)).toEqual([])
    await press(m, 'Decide…')
    expect(sheetsShown(m)).toEqual(['conflict'])
    expect(latest(m, sheets).props.entry.file_id).toBe('f1')
    latest(m, sheets).props.onClose()
    m.render()
    expect(sheetsShown(m)).toEqual([])
  })

  test('with two conflicts, each "Decide…" opens the sheet for its own file', async () => {
    const second = { ...conflict, id: 'conflict:f9', file_id: 'f9', file_name: 'Second.md' }
    const { m, sheets } = open(ready(conflict, second))
    const decide = () => buttons(m).filter((el) => buttonLabel(el) === 'Decide…')
    expect(decide()).toHaveLength(2)
    await decide()[1].props.onClick()
    m.render()
    expect(latest(m, sheets).props.entry.file_id).toBe('f9')
    latest(m, sheets).props.onClose()
    m.render()
    await decide()[0].props.onClick()
    m.render()
    expect(latest(m, sheets).props.entry.file_id).toBe('f1')
  })

  test('a resolved conflict asks the window to read the feed again, so the row leaves', async () => {
    const { m, sheets, events } = open(ready(conflict))
    await press(m, 'Decide…')
    latest(m, sheets).props.onResolved()
    expect(events).toEqual(['changed'])
  })

  test('"Add storage" opens the billing page in the browser', async () => {
    const { m, urls } = open(ready(quota))
    await press(m, 'Add storage')
    expect(urls).toEqual([desktopApi.BILLING_URL])
  })

  test('"Sign in again" starts the in-place sign-in and says nothing when it works', async () => {
    const calls: string[] = []
    const { m } = open(ready(signedOut), { bindings: { forceReauth: async () => { calls.push('reauth'); return { ok: true, value: { warning: null } } } } })
    await press(m, 'Sign in again')
    expect(calls).toEqual(['reauth'])
    expect(m.toasts).toEqual([])
  })

  test('a sign-in that cannot start is one error toast with the reason; a sign-in that did start with a warning shows it as info', async () => {
    const failing = open(ready(signedOut), { bindings: { forceReauth: async () => ({ ok: false, reason: 'no window', unsupported: false }) } })
    await press(failing.m, 'Sign in again')
    expect(failing.m.toasts).toEqual([{ variant: 'error', title: 'Couldn’t open sign-in', message: 'no window' }])
    const warned = open(ready(signedOut), { bindings: { forceReauth: async () => ({ ok: true, value: { warning: { sentence: 'We couldn’t confirm the old session ended.' } } }) } })
    await press(warned.m, 'Sign in again')
    expect(warned.m.toasts).toEqual([{ variant: 'info', message: 'We couldn’t confirm the old session ended.' }])
  })
})

describe('Version history… (the toolbar button lives in the window; the sheet is Review\'s)', () => {
  test('closed by default; open when the window says so, with the way to close it handed through', async () => {
    const closed = open(ready(conflict))
    expect(sheetsShown(closed.m)).toEqual([])
    const shown = open(ready(conflict), { historyOpen: true })
    expect(sheetsShown(shown.m)).toEqual(['history'])
    latest(shown.m, shown.sheets).props.onClose()
    expect(shown.events).toEqual(['history-closed'])
  })

  test('it is available when nothing needs a decision: finding an old version is not a Review item', async () => {
    const { m } = open(ready(), { historyOpen: true })
    expect(sheetsShown(m)).toEqual(['history'])
    await settle(m)
  })

  test('and over a list that failed to load, since it does not depend on the feed', () => {
    const { m } = open({ status: 'failed', entries: [], reason: 'x' }, { historyOpen: true })
    expect(sheetsShown(m)).toEqual(['history'])
  })

  test('it exposes no button of its own: one place for the button (the toolbar), not two', () => {
    const { m } = open(ready(conflict))
    expect(() => button(m, 'Version history…')).toThrow()
  })
})
