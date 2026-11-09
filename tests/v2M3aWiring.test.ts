// M3a: the wiring of B03 (global elements) and B04 (Files): the sidebar's problem badge, a failed action's words,
// type-to-select, and where each banner's button goes.
import { describe, expect, test } from 'bun:test'
import { rowCount } from '../src/v2/chrome/Sidebar'
import { actionFailure, fmtWhen, typeahead, TYPEAHEAD_MS } from '../src/v2/sections/files/model'
import { BANNER_ROUTE, bannerCopy, listAge } from '../src/v2/chrome/Banners'
import { statusWords, uploadingLine } from '../src/v2/chrome/StatusLine'
import { createFixtureExtras, FIXTURE_TRANSFERS } from '../src/v2/source/fixtures/extras'

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
    expect(actionFailure('rename', 'offline')).toEqual({ text: 'Couldn’t rename. Check your connection.', retry: true })
  })
  test('no Try again where trying again cannot help; never a code', () => {
    for (const code of ['unsupported', 'quota', 'signed-out', 'name-taken']) {
      const f = actionFailure('move it', code)
      expect(f.retry).toBe(false)
      expect(f.text.startsWith('Couldn’t move it. ')).toBe(true)
      expect(f.text).not.toContain(code)
    }
    expect(actionFailure('open it', 'failed')).toEqual({ text: 'Couldn’t open it.', retry: true })
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

describe('the status line says the state that needs attention (DW-0401, DW-0426, DW-1107 to DW-1109)', () => {
  const sync = { phase: 'up-to-date' as const, onDevice: 4, availableOffline: 3, onlineOnly: 4, pending: 0 }
  test('up to date: the counts', () => {
    expect(statusWords(sync)).toBe('Up to date · 4 on this PC · 3 available offline · 4 online only')
  })
  test('each attention state alone, in the board’s words', () => {
    expect(statusWords({ ...sync, phase: 'offline' })).toBe('Offline')
    expect(statusWords(sync, { signedOut: true, uploading: 'Uploading 2 of 4' })).toBe('Signed out')
    expect(statusWords(sync, { uploading: 'Uploading 2 of 4 · 42.4 MB left' })).toBe('Uploading 2 of 4 · 42.4 MB left')
    expect(statusWords(sync, { listing: true })).toBe('Getting your files ready')
  })
  test('offline or paused wins over queued uploads (self-review)', () => {
    expect(statusWords({ ...sync, phase: 'offline' }, { uploading: 'Uploading 1 of 3' })).toBe('Offline')
    expect(statusWords({ ...sync, phase: 'paused' }, { uploading: 'Uploading 1 of 3' }).startsWith('Paused · ')).toBe(true)
    expect(uploadingLine(FIXTURE_TRANSFERS.filter((r) => r.state !== 'moving'))).toBeNull()
  })
  test('uploads: moving of all, the bytes left of those not stopped; nothing moving is no line', () => {
    expect(uploadingLine(FIXTURE_TRANSFERS)).toBe('Uploading 2 of 4 · 52.7 MB left')
    expect(uploadingLine(FIXTURE_TRANSFERS.filter((r) => r.state === 'stopped'))).toBeNull()
    expect(uploadingLine([])).toBeNull()
  })
})

describe('the offline banner says how old the list is (DW-0101, DW-1108)', () => {
  const now = new Date('2026-11-08T12:00:00')
  test('today, yesterday, a date', () => {
    expect(listAge('2026-11-08T09:12:00', now)).toBe('today at 09:12')
    expect(listAge('2026-11-07T22:40:00', now)).toBe('yesterday at 22:40')
    expect(listAge('2026-11-03T08:00:00', now)).toBe('on 3 Nov at 08:00')
    expect(listAge('not a time', now)).toBe('before the connection dropped')
  })
  test('DW-1108’s sentence', () => {
    expect(bannerCopy('offline', { asOf: 'today at 09:12' }).detail).toBe('This list is from today at 09:12. Changes you make will upload when you’re back.')
  })
})

describe('calendar days across a DST change (run under TZ=Europe/Lisbon too)', () => {
  // Summer time ends on 25 Oct 2026: the two local midnights are 25 hours apart.
  const now = new Date('2026-10-26T09:00:00')
  test('the evening before is yesterday', () => {
    expect(listAge('2026-10-25T22:00:00', now)).toBe('yesterday at 22:00')
    expect(fmtWhen('2026-10-25T22:00:00', now)).toBe('Yesterday, 22:00')
    expect(fmtWhen('2026-10-24T22:00:00', now)).toBe('24 Oct 2026')
  })
})
