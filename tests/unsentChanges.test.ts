/**
 * Task 2409: the words of Activity › "Not sent" (`src/windows/unsentChanges.ts`), the loader's paging, and the wire
 * contract with Rust (`tests/fixtures/unsent-changes.page.json`, which `unsent_changes_commands.rs` serialises).
 */
import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { BILLING_URL, type CommandResult } from '../src/desktopApi'
import {
  UNSENT_MAX_PER_CALL,
  UNSENT_PAGE_SIZE,
  baseName,
  folderOf,
  keepRowStates,
  kindWord,
  notDownloadedAction,
  notDownloadedReason,
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
  blocked: null,
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

  test('reads the rows shown in one call (one read transaction), up to the most one call serves', async () => {
    const backend = scripted(250)
    const result = await loadUnsentChanges(300, backend.call)
    expect(result.ok && result.value.items.map((i) => i.id)).toEqual(Array.from({ length: 250 }, (_, i) => `op-${i}`))
    expect(backend.calls).toEqual([{ offset: 0, limit: 300 }])
    const first = scripted(250)
    const one = await loadUnsentChanges(UNSENT_PAGE_SIZE, first.call)
    expect(one.ok && [one.value.items.length, one.value.total]).toEqual([100, 250])
    expect(first.calls).toEqual([{ offset: 0, limit: 100 }])
    // Past what one call serves (`MAX_PAGE` in unsent_changes_commands.rs), the rest in further calls, and no change
    // twice: a change that moved between two calls shifts the order, and a row must not repeat (self-review).
    const many = scripted(1_200)
    const shown = await loadUnsentChanges(1_200, many.call)
    expect(many.calls).toEqual([
      { offset: 0, limit: UNSENT_MAX_PER_CALL },
      { offset: 500, limit: UNSENT_MAX_PER_CALL },
      { offset: 1_000, limit: 200 },
    ])
    expect(shown.ok && shown.value.items.length).toBe(1_200)
    const shifted = (async (_name: string, args?: Record<string, unknown>) => {
      const { offset, limit } = args as { offset: number; limit: number }
      // The second call starts one row early: the first call's last row comes again.
      const from = offset === 0 ? 0 : offset - 1
      return { ok: true, value: page(600, from, Math.min(limit, 600 - from)) }
    }) as unknown as typeof import('../src/desktopApi').command
    const unique = await loadUnsentChanges(600, shifted)
    const ids = unique.ok ? unique.value.items.map((i) => i.id) : []
    expect(new Set(ids).size).toBe(ids.length)
  })

  test('the most one call serves is the backend’s own clamp', () => {
    const rust = readFileSync(new URL('../src-tauri/src/unsent_changes_commands.rs', import.meta.url), 'utf8')
    expect(rust).toContain(`const MAX_PAGE: u32 = ${UNSENT_MAX_PER_CALL};`)
  })

  test('a failed or malformed page fails the read', async () => {
    const locked = (async () => ({ ok: false, reason: 'vault is locked', unsupported: false })) as unknown as typeof import('../src/desktopApi').command
    expect(await loadUnsentChanges(100, locked)).toEqual({ ok: false, reason: 'vault is locked', unsupported: false })
    const malformed = (async () => ({ ok: true, value: { total: 'x' } })) as unknown as typeof import('../src/desktopApi').command
    const result: CommandResult<UnsentChangesPage> = await loadUnsentChanges(100, malformed)
    expect(result.ok).toBe(false)
  })
})

// ── Task 2414 ─────────────────────────────────────────────────────────────────────────────────────────────────────

describe('task 2414: a failed change Try again would not send says why, and has no Try again', () => {
  test('the copy to send is gone; a newer version is already in Beebeeb', () => {
    expect(reason({ group: 'gave_up', blocked: 'copy_gone' })).toEqual({ text: 'Can’t send: the copy to send is gone.', attention: true })
    expect(reason({ group: 'gave_up', kind: 'edit', blocked: 'superseded' }).text).toBe('Can’t send: a newer version of this file is already in Beebeeb. Nothing was overwritten.')
    expect(reason({ group: 'gave_up', kind: 'new_file', blocked: 'superseded' }).text).toBe('Can’t send: this file is already in Beebeeb. Nothing was overwritten.')
    expect(reason({ group: 'gave_up', blocked: 'a-later-reason' })).toEqual({ text: 'Can’t send it as it is.', attention: true })
    expect(unsentAction({ ...base, group: 'gave_up', blocked: 'copy_gone' })).toBeNull()
  })

  test('one still backing off says when the engine tries it again (it does, all the same), with no Try again', () => {
    const item = { ...base, group: 'waiting', retry_at: new Date(2026, 10, 8, 14, 5).getTime() / 1000, blocked: 'superseded' }
    expect(reason(item)).toEqual({ text: 'Couldn’t send; trying again at 14:05', attention: true })
    expect(unsentAction(item)).toBeNull()
  })

  test('a refused Try again says why', () => {
    expect(retryRefusal('superseded')).toBe('A newer version of this file is already in Beebeeb, so this change isn’t sent over it. Nothing was overwritten.')
    expect(retryRefusal('copy_gone')).toBe('The copy to send is gone, so trying again can’t send it.')
  })

  test('the wire carries blocked as a word or null; anything else is no page', () => {
    const page = (blocked: unknown) => ({ ...fixture, items: [{ ...fixture.items[0], blocked }] })
    expect(parseUnsentChanges(page('copy_gone'))?.items[0].blocked).toBe('copy_gone')
    expect(parseUnsentChanges(page(null))?.items[0].blocked).toBeNull()
    expect(parseUnsentChanges(page(7))).toBeNull()
    const { blocked: _, ...without } = fixture.items[0]
    expect(parseUnsentChanges({ ...fixture, items: [without] })).toBeNull()
  })
})

describe('task 2414: "Not downloaded" rows', () => {
  test('a download is a kind of its own; a stopped or paused one reads as "Not sent" words it; no Try again', () => {
    expect(kindWord('download')).toBe('Download')
    const page = { ...fixture, items: [{ ...fixture.items[0], id: 'pin-1', kind: 'download', group: 'gave_up', retryable: false }] }
    const parsed = parseUnsentChanges(page)
    expect(parsed?.items[0].kind).toBe('download')
    const item = parsed!.items[0]
    expect(reason(item).text).toBe('Stopped after repeated errors')
    expect(unsentAction(item)).toBeNull()
    expect(reason({ ...item, group: 'auth' }).text).toBe('On hold until you sign in again')
  })

  test('its own words: never an upload’s sentence or action for a pause the queue never gives a download', () => {
    const row = { ...base, kind: 'download' as const }
    expect(notDownloadedReason({ ...row, group: 'gave_up' })).toEqual({ text: 'Stopped after repeated errors', attention: true })
    expect(notDownloadedReason({ ...row, group: 'auth' })).toEqual({ text: 'On hold until you sign in again', attention: true })
    expect(notDownloadedReason({ ...row, group: 'locked' })).toEqual({ text: 'On hold while Beebeeb is locked', attention: false })
    for (const group of ['quota', 'found_while_paused', 'permission', 'needs_review', 'other']) {
      expect(notDownloadedReason({ ...row, group })).toEqual({ text: 'On hold', attention: true })
      expect(notDownloadedAction({ ...row, group })).toBeNull()
    }
    expect(notDownloadedAction({ ...row, group: 'auth' })).toEqual({ kind: 'sign_in_again', label: 'Sign in again' })
    expect(notDownloadedAction({ ...row, group: 'gave_up' })).toBeNull()
  })

  test('the loader reads the list it is given', async () => {
    const names: string[] = []
    const call = (async (name: string) => {
      names.push(name)
      return { ok: true, value: { ...fixture, total: 0, items: [] } }
    }) as unknown as typeof import('../src/desktopApi').command
    await loadUnsentChanges(100, call, 'not_downloaded')
    await loadUnsentChanges(100, call)
    expect(names).toEqual(['not_downloaded', 'unsent_changes'])
    const malformed = (async () => ({ ok: true, value: { total: 'x' } })) as unknown as typeof import('../src/desktopApi').command
    expect(await loadUnsentChanges(100, malformed, 'not_downloaded')).toEqual({ ok: false, reason: 'The list of files not downloaded could not be read.', unsupported: false })
  })
})

describe('task 2414 (2409 review P3): "Queued" holds until a read shows the change moving', () => {
  const waiting: UnsentChange = { ...base, id: 'a', group: 'waiting' }
  test('kept while the change waits its turn; an action in flight always kept', () => {
    expect(keepRowStates({ a: 'queued', b: 'busy' }, [waiting])).toEqual({ a: 'queued', b: 'busy' })
  })
  test('dropped once it moves: sending, failed again, given up, held, blocked, or gone', () => {
    for (const moved of [
      { ...waiting, sending: { done: 1, total: 10 } },
      { ...waiting, retry_at: 99, retryable: true },
      { ...waiting, group: 'gave_up', retryable: true },
      { ...waiting, group: 'auth' },
      { ...waiting, blocked: 'copy_gone' },
    ]) {
      expect(keepRowStates({ a: 'queued' }, [moved])).toEqual({})
    }
    expect(keepRowStates({ a: 'queued' }, [])).toEqual({})
  })
})
