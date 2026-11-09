// M3a: the wiring of B03 (global elements) and B04 (Files): the sidebar's problem badge, a failed action's words,
// type-to-select, and where each banner's button goes.
import { describe, expect, test } from 'bun:test'
import { rowCount } from '../src/v2/chrome/Sidebar'
import { failureText, typeahead, TYPEAHEAD_MS } from '../src/v2/sections/files/model'
import { BANNER_ROUTE } from '../src/v2/chrome/Banners'
import { createFixtureExtras } from '../src/v2/source/fixtures/extras'

describe('the sidebar counts', () => {
  test('Sync history counts problems only: none at zero, red above it (DW-0401, DW-1301)', () => {
    expect(rowCount('sync-history', 0)).toBeNull()
    expect(rowCount('sync-history', 3)).toEqual({ text: '3', problems: true })
    expect(rowCount('sync-history', undefined)).toBeNull()
  })
  test('the other rows keep a plain count, zero included', () => {
    expect(rowCount('trash', 12)).toEqual({ text: '12', problems: false })
    expect(rowCount('shared', 0)).toEqual({ text: '0', problems: false })
  })
})

describe('the fixture world agrees with itself', () => {
  test('problems only where drive_status counts them', async () => {
    for (const scenario of ['default', 'offline', 'empty', 'syncing']) {
      const x = createFixtureExtras(scenario)
      expect({ scenario, listed: (await x.syncProblems()).length, counted: (await x.status()).syncProblems }).toEqual({ scenario, listed: 0, counted: 0 })
    }
    const p = createFixtureExtras('problems')
    expect((await p.syncProblems()).length).toBe((await p.status()).syncProblems)
  })
  test('the banner scenarios', async () => {
    expect((await createFixtureExtras('signed-out').status()).signedOut).toBe(true)
    expect((await createFixtureExtras('backup-problems').status()).backupProblems).toBe(3)
    expect((await createFixtureExtras('conflict').status()).conflicts).toEqual([{ id: 'i-contract', name: 'Contract draft.pdf' }])
  })
})

describe('a failed action says what failed (DW-0109)', () => {
  test('offline: the board’s words, and Try again can help', () => {
    expect(failureText('rename', 'offline')).toEqual({ text: 'Couldn’t rename. Check your connection.', retry: true })
  })
  test('no Try again where trying again cannot help; never a code', () => {
    for (const code of ['unsupported', 'quota', 'signed-out', 'name-taken']) {
      const f = failureText('move it', code)
      expect(f.retry).toBe(false)
      expect(f.text.startsWith('Couldn’t move it. ')).toBe(true)
      expect(f.text).not.toContain(code)
    }
    expect(failureText('open it', 'failed')).toEqual({ text: 'Couldn’t open it.', retry: true })
  })
})

describe('type-to-select builds a prefix (DW-0420b)', () => {
  const names = ['Archive 001.pdf', 'Archive 150 Quarterly report.pdf', 'Budget.xlsx', 'Day 1']
  test('letters typed together jump by the whole prefix', () => {
    let s = typeahead(null, 'a', 0, names)
    expect(s.hit).toBe(0)
    for (const [i, k] of [...'rchive 15'].entries()) s = typeahead(s, k, 100 + i * 100, names)
    expect(s).toEqual({ text: 'archive 15', at: 900, hit: 1 })
  })
  test('a pause starts a new prefix', () => {
    const first = typeahead(null, 'a', 0, names)
    expect(typeahead(first, 'b', TYPEAHEAD_MS + 1, names)).toEqual({ text: 'b', at: TYPEAHEAD_MS + 1, hit: 2 })
    expect(typeahead(first, 'z', 10, names).hit).toBe(-1)
  })
})

describe('the banners’ buttons (DW-0102 to DW-0106)', () => {
  test('each lands where the board solves it', () => {
    expect(BANNER_ROUTE['sync-problem']).toEqual({ section: 'sync-history', sub: 'problems' })
    expect(BANNER_ROUTE['storage-almost-full']).toEqual({ section: 'settings', sub: 'storage' })
    expect(BANNER_ROUTE['storage-full']).toEqual({ section: 'settings', sub: 'storage' })
    expect(BANNER_ROUTE['backup-problem']).toEqual({ section: 'settings', sub: 'backup-failures' })
    expect(BANNER_ROUTE['signed-out']).toBeUndefined()
  })
})
