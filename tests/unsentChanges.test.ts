/**
 * Task 2409: the words of Activity › "Not sent" (`src/windows/unsentChanges.ts`), the loader's paging, and the wire
 * contract with Rust (`tests/fixtures/unsent-changes.page.json`, which `unsent_changes_commands.rs` serialises).
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { BILLING_URL, type CommandResult } from '../src/desktopApi'
import {
  UNSENT_PAGE_SIZE,
  baseName,
  folderOf,
  kindWord,
  loadUnsentChanges,
  parseUnsentChanges,
  retryRefusal,
  retryTime,
  unsentAction,
  unsentReason,
  type UnsentChange,
  type UnsentChangesPage,
} from '../src/windows/unsentChanges'

const fixture = parseUnsentChanges(JSON.parse(readFileSync(new URL('./fixtures/unsent-changes.page.json', import.meta.url), 'utf8')))
if (!fixture) throw new Error('the Rust fixture did not parse')

const base: UnsentChange = {
  id: 'op',
  file_id: 'f',
  kind: 'new_file',
  path: 'docs/a.txt',
  backup: false,
  group: 'waiting',
  queued_at: 1,
  retry_at: null,
  parked: false,
  retryable: false,
  sending: null,
}
const idle = { engineState: 'idle', paused: false, now: new Date(2026, 10, 8, 13, 0) }
const reason = (patch: Partial<UnsentChange>, ctx = idle) => unsentReason({ ...base, ...patch }, ctx)

describe('the wire', () => {
  test('the Rust fixture parses, field for field', () => {
    expect(fixture.total).toBe(3)
    expect(fixture.items.map((i) => [i.kind, i.group, i.retryable])).toEqual([
      ['delete', 'gave_up', true],
      ['edit', 'quota', false],
      ['new_file', 'waiting', false],
    ])
    expect(fixture.items[2].sending).toEqual({ done: 34, total: 100 })
  })

  test('anything outside the shape is no page, never a guess', () => {
    for (const bad of [
      null,
      { ...fixture, total: -1 },
      { ...fixture, items: [{ ...fixture.items[0], kind: 'hydrate_file' }] },
      { ...fixture, items: [{ ...fixture.items[0], retryable: 'yes' }] },
      { ...fixture, paused: undefined },
    ]) {
      expect(parseUnsentChanges(bad)).toBeNull()
    }
  })
})

describe('why a change is not sent', () => {
  test('waiting: sending, a failure backing off, paused, before the first check, or queued', () => {
    expect(reason({ sending: { done: 34, total: 100 } })).toEqual({ text: 'Sending · 34%', attention: false })
    const at = Math.floor(new Date(2026, 10, 8, 14, 5).getTime() / 1000)
    expect(reason({ retry_at: at, retryable: true })).toEqual({ text: 'Couldn’t send; trying again at 14:05', attention: true })
    const tomorrow = Math.floor(new Date(2026, 10, 9, 9, 30).getTime() / 1000)
    expect(reason({ retry_at: tomorrow }).text).toBe('Couldn’t send; trying again at 9 Nov 09:30')
    expect(reason({}, { ...idle, paused: true }).text).toBe('Waits until you resume sync')
    expect(reason({}, { ...idle, engineState: 'running' }).text).toBe('Waiting for the first sync check')
    expect(reason({}).text).toBe('Queued')
  })

  test('every group the count has a sentence; a held one needs the person', () => {
    const cases: Array<[string, string, boolean]> = [
      ['waiting_for_folder', 'Waiting for its folder to reach Beebeeb', false],
      ['gave_up', 'Stopped after repeated errors', true],
      ['auth', 'On hold until you sign in again', true],
      ['quota', 'On hold: your storage is full', true],
      ['permission', 'On hold: you don’t have permission to make this change', true],
      ['locked', 'On hold while Beebeeb is locked', false],
      ['key_replaced', 'On hold: made with a key this account no longer uses', true],
      ['found_while_paused', 'Found while sync was paused; not uploaded until you decide', true],
      ['other', 'On hold', true],
    ]
    for (const [group, text, attention] of cases) expect(reason({ group })).toEqual({ text, attention })
    expect(reason({ group: 'needs_review' }).text).toStartWith('On hold: an item with the same name is already in Beebeeb.')
    expect(reason({ group: 'not_marked_synced' }).text).toStartWith('Uploaded, but this PC couldn’t mark it as synced.')
    expect(reason({ group: 'gave_up', parked: true }).text).toBe('Stopped: Beebeeb can’t send it as it is. Your copy stays on this PC.')
  })
})

describe('the one action', () => {
  test('Try again only on a change that failed, and not while it is being sent', () => {
    expect(unsentAction({ ...base, group: 'gave_up', retryable: true })).toEqual({ kind: 'retry', label: 'Try again' })
    expect(unsentAction({ ...base, retry_at: 99, retryable: true })).toEqual({ kind: 'retry', label: 'Try again' })
    expect(unsentAction({ ...base, group: 'gave_up', parked: true })).toBeNull()
    expect(unsentAction({ ...base, retryable: true, sending: { done: 1, total: 2 } })).toBeNull()
    expect(unsentAction(base)).toBeNull()
  })

  test('a held change keeps its own action and never gets Try again', () => {
    expect(unsentAction({ ...base, group: 'auth' })).toEqual({ kind: 'sign_in_again', label: 'Sign in again' })
    expect(unsentAction({ ...base, group: 'quota' })).toEqual({ kind: 'add_storage', label: 'Add storage', href: BILLING_URL })
    expect(unsentAction({ ...base, group: 'found_while_paused' })).toEqual({ kind: 'open_sync_settings', label: 'Decide in Settings › Sync' })
    for (const group of ['permission', 'locked', 'key_replaced', 'needs_review', 'not_marked_synced', 'waiting_for_folder']) {
      expect(unsentAction({ ...base, group })).toBeNull()
    }
  })

  test('a refused Try again says why', () => {
    expect(retryRefusal('unknown')).toBe('It isn’t waiting any more: it was sent or removed in the meantime.')
    expect(retryRefusal('held')).toStartWith('It’s on hold')
    expect(retryRefusal('parked')).toStartWith('Beebeeb can’t send this one as it is')
    expect(retryRefusal('network down')).toBe('network down')
  })
})

describe('names', () => {
  test('a row is its file name, its folder under it, and its kind word', () => {
    expect([baseName('Projects/2026/old plan.docx'), folderOf('Projects/2026/old plan.docx')]).toEqual(['old plan.docx', 'Projects/2026'])
    expect([baseName('top.txt'), folderOf('top.txt')]).toEqual(['top.txt', ''])
    expect([baseName('C:\\Users\\me\\Pictures\\a.jpg'), folderOf('C:\\Users\\me\\Pictures\\a.jpg')]).toEqual(['a.jpg', 'C:\\Users\\me\\Pictures'])
    expect(['new_file', 'edit', 'folder', 'rename', 'move', 'delete', 'restore'].map((k) => kindWord(k as UnsentChange['kind']))).toEqual([
      'New file', 'Edit', 'Folder', 'Rename', 'Move', 'Delete', 'Restore',
    ])
    expect(retryTime(Math.floor(new Date(2026, 10, 8, 7, 4).getTime() / 1000), idle.now)).toBe('07:04')
  })
})

describe('the loader', () => {
  const page = (total: number, offset: number, count: number): UnsentChangesPage => ({
    ...fixture,
    total,
    items: Array.from({ length: count }, (_, i) => ({ ...base, id: `op-${offset + i}` })),
  })
  function scripted(total: number) {
    const calls: Array<{ offset: number; limit: number }> = []
    const call = (async (_name: string, args?: Record<string, unknown>) => {
      const { offset, limit } = args as { offset: number; limit: number }
      calls.push({ offset, limit })
      return { ok: true, value: page(total, offset, Math.max(0, Math.min(limit, total - offset))) }
    }) as unknown as typeof import('../src/desktopApi').command
    return { calls, call }
  }

  test('reads the rows shown a page at a time and stops at the total', async () => {
    const backend = scripted(250)
    const result = await loadUnsentChanges(300, backend.call)
    expect(result.ok && result.value.items.map((i) => i.id)).toEqual(Array.from({ length: 250 }, (_, i) => `op-${i}`))
    expect(backend.calls).toEqual([0, 100, 200].map((offset) => ({ offset, limit: UNSENT_PAGE_SIZE })))
    const first = scripted(250)
    const one = await loadUnsentChanges(100, first.call)
    expect(one.ok && [one.value.items.length, one.value.total]).toEqual([100, 250])
    expect(first.calls).toHaveLength(1)
  })

  test('a failed or malformed page fails the read', async () => {
    const locked = (async () => ({ ok: false, reason: 'vault is locked', unsupported: false })) as unknown as typeof import('../src/desktopApi').command
    expect(await loadUnsentChanges(100, locked)).toEqual({ ok: false, reason: 'vault is locked', unsupported: false })
    const malformed = (async () => ({ ok: true, value: { total: 'x' } })) as unknown as typeof import('../src/desktopApi').command
    const result: CommandResult<UnsentChangesPage> = await loadUnsentChanges(100, malformed)
    expect(result.ok).toBe(false)
  })
})
