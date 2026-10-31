/**
 * The version-history sheet (task 1683 slice 4; spec §7.6 "Version history… (toolbar button): a sheet that searches
 * file names and lists versions with 'Restore'", §10.2 rows 14 and 15): the real `VersionHistorySheet` declaration
 * against a scripted backend. The commands are the ones the Windows dialog and the Linux Versions page use
 * (`desktop_search_files`, `list_file_versions`, `restore_file_version`); this is the one Mac implementation of both.
 * What this proves: search, pick, list, restore, and every failure in its words; that restoring never claims more than
 * "queued". What it does NOT prove: that the server restores (a device check), layout.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import { actionNamedFor } from '../src/actionName'
import * as parts from '../src/macSettingsParts'
import * as reviewModel from '../src/reviewModel'
import { formatStorageSize } from '../src/storageFormat'
import { mount, type Mounted } from './fixtures/componentHarness'
import { button, buttons, buttonLabel, find, lines, modalNow, modalStub, press, settle, visibleLabel } from './fixtures/sheetHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const hit = (id: string, name: string, rel: string) => ({ file_id: id, name, rel_path: rel, status: 'synced', size_bytes: 48_000, modified_at: 1_790_000_000, shared_with_me: false })
const search = (results = [hit('f1', 'Q3 ledger.xlsx', 'Investigations/Q3 ledger.xlsx'), hit('f2', 'Q3 notes.md', 'Q3 notes.md')]) =>
  ({ query: 'q3', index_state: 'ready', indexed_file_count: 2, results })
const versions = (extra: any[] = []) => ({
  file_id: 'f1', current_version: 5,
  versions: [
    { id: 'v5', version_number: 5, is_current: true, created_at: '2026-10-25T08:00:00Z', size_bytes: 52_000, restorable: true },
    { id: 'v4', version_number: 4, is_current: false, created_at: '2026-10-24T08:00:00Z', size_bytes: 48_000, restorable: true },
    { id: 'v3', version_number: 3, is_current: false, created_at: '2026-10-12T08:00:00Z', size_bytes: 44_000, restorable: true },
    ...extra,
  ],
})

function open(backend: Record<string, (args: any) => unknown> = {}) {
  const modal = modalStub()
  const closed: string[] = []
  const m = mount('VersionHistorySheet.tsx', 'VersionHistorySheet', {
    expand: true,
    backend: { desktop_search_files: () => search(), list_file_versions: () => versions(), restore_file_version: () => ({ status: 'queued' }), ...backend } as any,
    props: { onClose: () => closed.push('closed') },
    bindings: {
      ...desktopApi, ...reviewModel, formatStorageSize, actionNamedFor,
      Btn: parts.Btn, Note: parts.Note, SettingsIcon: parts.SettingsIcon,
      Modal: modal.Modal,
      SEARCH_LIMIT: 12, SEARCH_DEBOUNCE_MS: 120,
    },
  })
  mounted.push(m)
  return { m, modal, closed }
}
const field = (m: Mounted) => find(m, (el) => el.type === 'input')[0]
const type = async (m: Mounted, value: string) => { field(m).props.onChange({ target: { value }, currentTarget: { value } }); m.render(); await settle(m) }
const calls = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name).map((c) => c.args)

describe('the sheet', () => {
  test('is named "Version history", 520 pt, opens on its search field (typing is what it is for), and nothing is searched yet', () => {
    const { m, modal } = open()
    expect(modalNow(m, modal)).toMatchObject({ open: true, title: 'Version history', maxWidth: 520 })
    expect(modalNow(m, modal).initialFocusRef).toBeDefined()
    expect(field(m).props['aria-label']).toBe('Search file names')
    expect(calls(m, 'desktop_search_files')).toHaveLength(0)
  })

  test('Close (and Esc, which is the same handler) hands control back', () => {
    const { m, modal, closed } = open()
    modalNow(m, modal).onClose()
    expect(closed).toEqual(['closed'])
    void m
  })
})

describe('searching file names', () => {
  test('typing searches the vault by name with the shared limit, once the typing settles', async () => {
    const { m } = open()
    await type(m, 'q3')
    expect(calls(m, 'desktop_search_files')).toEqual([{ query: 'q3', limit: 12, forFinder: undefined }])
  })

  test('each hit is a button with the name and the folder path in mono', async () => {
    const { m } = open()
    await type(m, 'q3')
    expect(buttons(m).map(buttonLabel)).toEqual(expect.arrayContaining(['Q3 ledger.xlsx Investigations/Q3 ledger.xlsx', 'Q3 notes.md']))
  })

  test('spaces around the query are not part of it, and an empty query searches nothing', async () => {
    const { m } = open()
    await type(m, '   ')
    expect(calls(m, 'desktop_search_files')).toHaveLength(0)
    await type(m, '  q3 ')
    expect(calls(m, 'desktop_search_files')[0].query).toBe('q3')
  })

  test('an answer to an older query never lands over the newer one', async () => {
    const waiting: Record<string, (v: unknown) => void> = {}
    const { m } = open({ desktop_search_files: (args: any) => new Promise((resolve) => { waiting[args.query] = resolve }) })
    await type(m, 'q3')
    await type(m, 'zz')
    waiting.zz(search([hit('z1', 'Zebra.md', 'Zebra.md')]))
    await settle(m)
    waiting.q3(search())
    await settle(m)
    const names = buttons(m).map(buttonLabel)
    expect(names).toContain('Zebra.md')
    expect(names).not.toContain('Q3 notes.md')
  })

  test('no match, an index that is still filling, and a search that fails each say their own sentence', async () => {
    const none = open({ desktop_search_files: () => search([]) })
    await type(none.m, 'zzz')
    expect(lines(none.m)).toContain('No matching files.')
    const filling = open({ desktop_search_files: () => ({ ...search([]), index_state: 'syncing' }) })
    await type(filling.m, 'zzz')
    expect(lines(filling.m)).toContain('Search index is syncing.')
    const failing = open({ desktop_search_files: () => { throw new Error('index unreadable') } })
    await type(failing.m, 'zzz')
    expect(lines(failing.m)).toContain('index unreadable')
  })
})

describe('the versions of one file', () => {
  async function pick(backend: Record<string, (args: any) => unknown> = {}) {
    const o = open(backend)
    await type(o.m, 'q3')
    await press(o.m, 'Q3 ledger.xlsx Investigations/Q3 ledger.xlsx')
    return o
  }

  test('picking a hit asks for THAT file\'s versions and shows the earlier ones, newest first, each with Restore', async () => {
    const { m } = await pick()
    expect(calls(m, 'list_file_versions')).toEqual([{ fileId: 'f1' }])
    const text = lines(m).join(' | ')
    expect(text).toContain('Q3 ledger.xlsx')
    expect(text).toContain('Version 4')
    expect(text).toContain('Version 3')
    expect(text.indexOf('Version 4')).toBeLessThan(text.indexOf('Version 3'))
    expect(buttons(m).filter((el) => visibleLabel(el) === 'Restore')).toHaveLength(2)
  })

  test('each Restore is named for its version and its file, with the visible word first; the one under way loses the name (it reads "Restoring…")', async () => {
    const { m } = await pick()
    const restores = () => buttons(m).filter((el) => visibleLabel(el) === 'Restore')
    expect(restores().map(buttonLabel)).toEqual(['Restore, Version 4, Q3 ledger.xlsx', 'Restore, Version 3, Q3 ledger.xlsx'])
    for (const el of restores()) expect(buttonLabel(el).startsWith('Restore')).toBe(true)
  })

  test('the current version is named in the summary and has no Restore: you cannot restore what you already have', async () => {
    const { m } = await pick()
    expect(lines(m).join(' | ')).toContain('Version 5 is current')
    expect(buttons(m).filter((el) => visibleLabel(el) === 'Restore')).toHaveLength(2)
  })

  test('a row says when and how big, in the Mac\'s units, and never prints an internal id', async () => {
    const { m } = await pick()
    const text = lines(m).join(' | ')
    expect(text).toContain(formatStorageSize(48_000))
    expect(text).not.toContain('v4')
    expect(text).not.toContain('object_version')
  })

  test('Restore asks the engine to restore that version and says "queued", not "restored"', async () => {
    const { m } = await pick()
    await m.clickNoWait('Restore')
    await settle(m)
    expect(calls(m, 'restore_file_version')).toEqual([{ fileId: 'f1', versionId: 'v4' }])
    expect(m.toasts).toEqual([{ variant: 'success', title: 'Restore queued', message: 'Q3 ledger.xlsx goes back to version 4 on the next sync.' }])
    for (const toast of m.toasts) expect(JSON.stringify(toast)).not.toContain('sync engine')
  })

  test('while a restore is being queued its button says so and no other restore can start', async () => {
    let release: () => void = () => {}
    const { m } = await pick({ restore_file_version: () => new Promise((resolve) => { release = () => resolve({ status: 'queued' }) }) })
    await m.clickNoWait('Restore')
    const labels = buttons(m).map(buttonLabel)
    expect(labels).toContain('Restoring…')
    expect(buttons(m).filter((el) => visibleLabel(el) === 'Restore').every((el) => el.props.disabled === true)).toBe(true)
    release()
    await settle(m)
  })

  test('a failed restore is one error toast with the reason, and the list stays', async () => {
    const { m } = await pick({ restore_file_version: () => { throw new Error('version is gone') } })
    await m.clickNoWait('Restore')
    await settle(m)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Restore failed' })
    expect(String(m.toasts[0].message)).toContain('version is gone')
    expect(lines(m).join(' | ')).toContain('Version 3')
  })

  test('versions that cannot be listed say the shared sentence and offer Back; the sheet does not close', async () => {
    const { m, closed } = await pick({ list_file_versions: () => { throw new Error('502') } })
    expect(lines(m)).toContain('Couldn’t load this file’s versions. Try again.')
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(1)
    expect(closed).toEqual([])
    await press(m, 'Back')
    expect(field(m)).toBeDefined()
  })

  test('a file with nothing earlier says so', async () => {
    const { m } = await pick({ list_file_versions: () => ({ file_id: 'f1', current_version: 1, versions: [{ id: 'v1', version_number: 1, is_current: true, created_at: null, size_bytes: 10 }] }) })
    expect(lines(m)).toEqual(expect.arrayContaining(['No earlier versions', 'Q3 ledger.xlsx has no earlier versions to go back to.']))
    expect(buttons(m).filter((el) => visibleLabel(el) === 'Restore')).toHaveLength(0)
  })

  test('Back returns to the search with the query still there', async () => {
    const { m } = await pick()
    await press(m, 'Back')
    expect(field(m).props.value).toBe('q3')
  })

  test('the versions of a file picked first never appear under the file picked second', async () => {
    const waiting: Record<string, (v: unknown) => void> = {}
    const o = open({ list_file_versions: (args: any) => new Promise((resolve) => { waiting[args.fileId] = resolve }) })
    await type(o.m, 'q3')
    void button(o.m, 'Q3 ledger.xlsx Investigations/Q3 ledger.xlsx').props.onClick()
    o.m.render()
    await settle(o.m)
    await press(o.m, 'Back')
    void button(o.m, 'Q3 notes.md').props.onClick()
    o.m.render()
    await settle(o.m)
    waiting.f1(versions())
    await settle(o.m)
    expect(lines(o.m).join(' | ')).not.toContain('Version 4')
    expect(lines(o.m)).toContain('Loading versions…')
    waiting.f2({ file_id: 'f2', current_version: 2, versions: [{ id: 'n2', version_number: 2, is_current: true, size_bytes: 3 }, { id: 'n1', version_number: 1, is_current: false, size_bytes: 2 }] })
    await settle(o.m)
    expect(lines(o.m).join(' | ')).toContain('Version 1')
    expect(lines(o.m).join(' | ')).not.toContain('Version 4')
  })

  test('an answer for a file the person has since left is dropped', async () => {
    let release: (v: unknown) => void = () => {}
    const o = open({ list_file_versions: () => new Promise((resolve) => { release = resolve }) })
    await type(o.m, 'q3')
    void button(o.m, 'Q3 ledger.xlsx Investigations/Q3 ledger.xlsx').props.onClick()
    o.m.render()
    await press(o.m, 'Back')
    release(versions())
    await settle(o.m)
    expect(lines(o.m).join(' | ')).not.toContain('Version 4')
  })
})

describe('copy that moved with the Windows dialog\'s wording', () => {
  test('no developer words and no raw ids on the sheet', async () => {
    const { m } = open()
    await type(m, 'q3')
    await press(m, 'Q3 ledger.xlsx Investigations/Q3 ledger.xlsx')
    const text = lines(m).join(' | ')
    for (const banned of ['sync engine', 'Queue restore', 'Version id', 'Object', 'daemon']) expect(text).not.toContain(banned)
    void button
  })
})
