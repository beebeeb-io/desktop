// M3c: the wiring of B05 (Shared, Search, Trash) and B14 (account settings). Each test names the board frame whose
// action it pins; each was seen red on a mutant of the code it guards (the PR body lists them).
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement } from 'react'
import { DriveSourceError, type TrashEntry } from '@beebeeb/shared/drive-ui'
import { restoredText } from '../src/v2/sections/trash'
import { openFailureText } from '../src/v2/sections/shared'
import { signOutRefusal, windowAfterSignOut } from '../src/v2/sections/settings/account/SecurityPane'
import { copyFor60s, COPY_DONE, COPY_FAILED } from '../src/v2/sections/settings/account/data'
import { SearchView } from '../src/v2/sections/search/SearchView'
import { createBoardSource } from '../src/v2/source/fixtures/board'
import { createFixtureExtras } from '../src/v2/source/fixtures/extras'
import { elementsOf, mount } from './fixtures/componentHarness'

const entry = (id: string, originalPath: string): TrashEntry => ({
  item: { id, parentId: null, name: `${id}.pdf`, isFolder: false, size: 1, modifiedAt: '2026-11-01T10:00:00', shared: { kind: 'none' }, local: 'online-only', hasThumbnail: false },
  trashedAt: '2026-11-07T17:22:00', originalPath,
})

describe('Trash › Restore says where it went (DW-0748)', () => {
  test('one item names its folder; a missing folder reads Files; several are counted', () => {
    expect(restoredText([entry('a', 'Documents')])).toBe('Restored to Documents')
    expect(restoredText([entry('a', '')])).toBe('Restored to Files')
    expect(restoredText([entry('a', 'Documents'), entry('b', 'Photos'), entry('c', 'Files')])).toBe('3 items restored')
  })
})

describe('Shared › a failed open, Details or offline switch says what failed (DW-0501, DW-0505)', () => {
  test('each DriveSource code has its own words, never the host text', () => {
    const cases: [string, string][] = [
      ['offline', 'Couldn’t open it. You’re offline.'],
      ['unsupported', 'Couldn’t open it. This isn’t available in this version of Beebeeb.'],
      ['not-found', 'Couldn’t open it. It isn’t there anymore.'],
      ['failed', 'Couldn’t open it. Try again.'],
    ]
    let checked = 0
    for (const [code, words] of cases) {
      expect(openFailureText(new DriveSourceError(code as never, 'drive_open: secret host detail'), 'open it')).toBe(words)
      checked += 1
    }
    expect(checked).toBe(4)
    expect(openFailureText(new Error('raw'), 'show the details')).toBe('Couldn’t show the details. Try again.')
  })
})

describe('Sign out lands on the real sign-in, never a board frame (DW-A57, DW-A57a)', () => {
  test('the window opens again as itself, without the section or a frame', () => {
    const next = windowAfterSignOut('?window=main-app&platform=windows&section=settings&sub=security&theme=dark&frame=DW-A02')
    expect(next).toBe('?window=main-app&platform=windows&theme=dark')
    expect(next).not.toContain('frame=')
    expect(next).not.toContain('section=')
  })
  test('a refused sign-out shows Rust’s own sentence, and anything else the generic words', () => {
    expect(signOutRefusal('unsent_changes: Some changes haven’t reached the server yet.')).toBe('Some changes haven’t reached the server yet.')
    expect(signOutRefusal('network')).toBe('Beebeeb can’t reach the server. Check the connection and try again.')
    expect(signOutRefusal('reqwest: connection refused at 10.0.0.1')).toBe('Something went wrong. Nothing was changed.')
  })
})

describe('Copy all codes and Copy key never say Copied when nothing was copied (DW-A69, DW-A70)', () => {
  const withClipboard = async (writeText: (t: string) => Promise<void>, run: () => Promise<boolean>) => {
    const nav = globalThis.navigator as unknown as Record<string, unknown>
    const before = Object.getOwnPropertyDescriptor(nav, 'clipboard')
    const w = globalThis as unknown as Record<string, unknown>
    const hadWindow = 'window' in w
    const prevWindow = w.window
    Object.defineProperty(nav, 'clipboard', { value: { writeText, readText: async () => '' }, configurable: true })
    w.window = { setTimeout: () => 0 }
    try { return await run() } finally {
      if (before) Object.defineProperty(nav, 'clipboard', before)
      else delete nav.clipboard
      if (hadWindow) w.window = prevWindow
      else delete w.window
    }
  }
  test('a refused clipboard answers false; a taken one true', async () => {
    expect(await withClipboard(async () => { throw new Error('denied') }, () => copyFor60s('1234 5678'))).toBe(false)
    expect(await withClipboard(async () => {}, () => copyFor60s('1234 5678'))).toBe(true)
    expect(COPY_DONE).toContain('Copied')
    expect(COPY_FAILED).not.toContain('Copied')
  })
})

describe('Move to a new authenticator sends the code the person typed (DW-A71 → DW-A69)', () => {
  test('the code dialog’s code reaches onMove', async () => {
    const moved: string[] = []
    const Stub = (p: { children?: unknown }) => createElement('div', null, p.children as never)
    const CodeDialog = (p: unknown) => createElement('code-dialog', p as never)
    const m = mount('v2/sections/settings/account/Factors.tsx', 'TotpStatus', {
      backend: {},
      bindings: { useEscBack() {}, Page: Stub, Group: Stub, Row: Stub, Pill: Stub, BackLink: Stub, CodeDialog, ago: () => '', day: () => '' },
      props: {
        state: { totp: { on: true, addedAt: null, lastUsedAt: null, codesLeft: 6, codesTotal: 8 } },
        dialog: 'move', onBack() {}, onNewCodes() {}, onTurnedOff() {}, onMove: (code: string) => moved.push(code),
      },
    })
    m.render()
    const dialog = elementsOf(m.tree()).find((e) => e.type === CodeDialog)
    expect(dialog?.props.kind).toBe('move')
    dialog!.props.onOk('482913')
    expect(moved).toEqual(['482913'])
  })
})

describe('Search: ↓ from the field selects the result it moves to (DW-0802)', () => {
  const hit = (id: string, name: string) => ({ item: { id, parentId: null, name, isFolder: false, size: 1, modifiedAt: '2026-11-07T10:12:00', shared: { kind: 'none' as const }, local: 'downloaded' as const, hasThumbnail: false }, where: 'Files' })
  test('the selected row is aria-selected and named by its name and where it is', () => {
    const tree = SearchView({ query: 'Contr', hits: [hit('a', 'Contract draft.pdf'), hit('b', 'Contractors.pdf')], selected: 'b' })
    const rows = elementsOf(tree).filter((e) => e.props?.['data-result'])
    expect(rows.map((r) => [r.props['data-result'], r.props['aria-selected'], r.props['aria-label']])).toEqual([
      ['a', false, 'Contract draft.pdf, Files'],
      ['b', true, 'Contractors.pdf, Files'],
    ])
  })
})

describe('the fixture world for B05 (instrument inputs)', () => {
  test('no-shares: nothing shared, while Files keeps DW-0401’s rows (DW-0509 › Files)', async () => {
    const s = createBoardSource('no-shares')
    expect((await s.shared()).length).toBe(0)
    expect((await s.list(null)).items.length).toBeGreaterThan(0)
  })
  test('a shared item has Details and versions (DW-0505 › Details)', async () => {
    const s = createBoardSource()
    const d = await s.details('sw-brand')
    expect([d.item.name, d.path.map((c) => c.name).join(' › '), d.versionCount]).toEqual(['Brand review.pdf', 'Shared with you', 3])
    expect((await s.versions('sw-brand')).length).toBe(3)
  })
  test('each search hit says where it is, as DW-0802 draws it', async () => {
    const where = await createFixtureExtras('default').paths(['s-contract-draft', 's-contract-2025', 's-contractors'])
    expect(where).toEqual({ 's-contract-draft': 'Files', 's-contract-2025': 'Documents › Legal', 's-contractors': 'Documents' })
  })
})

describe('Delete step 3 and 4 claim nothing this build cannot see (DW-A81, DW-A82)', () => {
  const src = readFileSync(new URL('../src/v2/sections/settings/account/SecurityPane.tsx', import.meta.url), 'utf8')
  const account = readFileSync(new URL('../src/v2/sections/settings/account/AccountPane.tsx', import.meta.url), 'utf8')
  test('no "moves on by itself", no guessed 72 hours, and Send again never only restarts a countdown', () => {
    expect(src).not.toContain('moves on by itself')
    expect(src).not.toContain('72 hours after the link was used')
    expect(src).not.toContain('onClick={() => { setWait(60) }}')
    expect(account).not.toContain('onClick={() => { setWait(60) }}')
  })
})
