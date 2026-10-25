/**
 * The Trash pane (task 1683 slice 4; spec §7.7: "From the Windows shell, copy unchanged"; hi-fi section 3): the real
 * `TrashPane` declaration against a scripted backend. Ported from `WindowsApp.tsx` `TrashView`: the same three
 * commands (`desktop_trash_list`, `desktop_trash_restore`, then `desktop_confirm_action` and
 * `desktop_trash_delete_permanently` behind a password sheet), in the Mac window's rows and sheet.
 * What this proves: the four states, what each button sends, that a permanent delete cannot happen without the
 * password's confirmation token, that a wrong password keeps the sheet and says so once, and that Refresh reads again.
 * What it does NOT prove: that the server restores or deletes (device checks), layout.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import * as desktopApi from '../src/desktopApi'
import * as parts from '../src/macSettingsParts'
import { formatStorageSize } from '../src/storageFormat'
import * as trashModel from '../src/trashModel'
import { mount, type Mounted } from './fixtures/componentHarness'
import { button, buttons, buttonLabel, find, lines, modalNow, modalStub, press, pressFirst, settle } from './fixtures/sheetHarness'

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

const yesterdaySeconds = () => { const d = new Date(); d.setDate(d.getDate() - 1); d.setHours(12, 0, 0, 0); return Math.floor(d.getTime() / 1000) }
const files = () => [
  { id: 't1', name: 'Draft 3, interview questions.docx', is_folder: false, size_bytes: 48_000, updated_at: String(yesterdaySeconds()) },
  { id: 't2', name: 'Old scans', is_folder: true, size_bytes: 0, updated_at: String(yesterdaySeconds()) },
]

function open(backend: Record<string, (args: any) => unknown> = {}, refreshSignal = 0) {
  const modal = modalStub()
  const props = { refreshSignal }
  const m = mount('TrashPane.tsx', 'TrashPane', {
    expand: true,
    backend: {
      desktop_trash_list: () => files(),
      desktop_trash_restore: () => null,
      desktop_confirm_action: () => ({ confirmation_token: 'tok-1', expires_at: '2099-01-01T00:00:00Z' }),
      desktop_trash_delete_permanently: () => null,
      ...backend,
    } as any,
    props,
    bindings: {
      ...desktopApi, ...trashModel, formatStorageSize,
      Btn: parts.Btn, Note: parts.Note, SettingsIcon: parts.SettingsIcon,
      Modal: modal.Modal,
    },
  })
  mounted.push(m)
  return { m, modal, props }
}
const calls = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name).map((c) => c.args)
const rowsOf = (m: Mounted) => find(m, (el) => el.props['data-row'] === 'trash')
const password = (m: Mounted) => find(m, (el) => el.type === 'input' && el.props.type === 'password')[0]
const typePassword = async (m: Mounted, value: string) => { password(m).props.onChange({ target: { value }, currentTarget: { value } }); m.render(); await settle(m) }

describe('states', () => {
  test('loading, then the intro and one row per item', async () => {
    const { m } = open()
    expect(lines(m)).toEqual(['Loading…'])
    await settle(m)
    expect(lines(m)[0]).toBe('Deleted files in your vault. Restore puts them back through sync; permanent delete removes them from Trash.')
    expect(rowsOf(m)).toHaveLength(2)
  })

  test('a row: name, kind · size · when, Restore and Delete…', async () => {
    const { m } = open()
    await settle(m)
    const flat = (row: any) => lines({ ...m, tree: () => row } as Mounted)
    expect(flat(rowsOf(m)[0])).toEqual(['Draft 3, interview questions.docx', `File · ${formatStorageSize(48_000)} · deleted yesterday`, 'Restore', 'Delete…'])
    expect(flat(rowsOf(m)[1])).toEqual(['Old scans', 'Folder · deleted yesterday', 'Restore', 'Delete…'])
  })

  test('empty: "Trash is empty" and what will appear', async () => {
    const { m } = open({ desktop_trash_list: () => [] })
    await settle(m)
    expect(lines(m)).toEqual(expect.arrayContaining(['Trash is empty', 'Deleted files will appear here.']))
    expect(rowsOf(m)).toHaveLength(0)
  })

  test('a list that cannot load is one alert with the reason and Try again that loads once more', async () => {
    let fail = true
    const { m } = open({ desktop_trash_list: () => { if (fail) throw new Error('503 from the server'); return files() } })
    await settle(m)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(1)
    expect(lines(m)).toEqual(expect.arrayContaining(['Couldn’t load Trash', '503 from the server']))
    fail = false
    await press(m, 'Try again')
    expect(rowsOf(m)).toHaveLength(2)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
  })

  test('the Windows shell\'s dash for an unknown time and its bare "—" size are gone', async () => {
    const { m } = open({ desktop_trash_list: () => [{ id: 't9', name: 'x.txt', is_folder: false, size_bytes: 1, updated_at: '' }] })
    await settle(m)
    expect(lines(m).join(' | ')).not.toContain('—')
  })
})

describe('Restore', () => {
  test('sends that item\'s id and name, and the row leaves with no toast (the Windows shell\'s behaviour)', async () => {
    const { m } = open()
    await settle(m)
    await pressFirst(m, 'Restore')
    expect(calls(m, 'desktop_trash_restore')).toEqual([{ fileId: 't1', fileName: 'Draft 3, interview questions.docx' }])
    expect(rowsOf(m)).toHaveLength(1)
    expect(lines(m).join(' | ')).not.toContain('Draft 3')
    expect(m.toasts).toEqual([])
  })

  test('a failed restore is one error toast, and the list is not wiped by it', async () => {
    const { m } = open({ desktop_trash_restore: () => { throw new Error('parent folder is gone') } })
    await settle(m)
    await pressFirst(m, 'Restore')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Restore failed' })
    expect(String(m.toasts[0].message)).toContain('parent folder is gone')
    expect(rowsOf(m)).toHaveLength(2)
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
  })

  test('while one restores, both of that row\'s buttons are off', async () => {
    let release: () => void = () => {}
    const { m } = open({ desktop_trash_restore: () => new Promise<null>((resolve) => { release = () => resolve(null) }) })
    await settle(m)
    await m.clickNoWait('Restore')
    const inRow = (n: any): any[] => (Array.isArray(n) ? n.flatMap(inRow) : n?.props ? [...(n.type === 'button' ? [n] : []), ...inRow(n.props.children)] : [])
    const [first, second] = rowsOf(m)
    expect(inRow(first).map(buttonLabel)).toEqual(['Restore', 'Delete…'])
    expect(inRow(first).every((b) => b.props.disabled === true)).toBe(true)
    expect(inRow(second).every((b) => b.props.disabled !== true)).toBe(true)
    release()
    await settle(m)
  })
})

describe('Delete… (permanent: a password first)', () => {
  test('opens the "Delete permanently" sheet naming the file, on its close button, with nothing sent yet', async () => {
    const { m, modal } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    expect(modalNow(m, modal)).toMatchObject({ open: true, title: 'Delete permanently' })
    expect(modalNow(m, modal).initialFocusRef).toBeUndefined()
    expect(lines(m)).toContain('Draft 3, interview questions.docx will be removed from Trash. This cannot be undone.')
    expect(calls(m, 'desktop_confirm_action')).toHaveLength(0)
    expect(calls(m, 'desktop_trash_delete_permanently')).toHaveLength(0)
  })

  test('the red confirm is off until a password is typed, and is the only red button', async () => {
    const { m } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    const confirm = () => button(m, 'Delete permanently')
    expect(confirm().props.disabled).toBe(true)
    expect(String(confirm().props.className)).toContain('ms-btn--danger')
    expect(buttons(m).filter((el) => String(el.props.className).includes('ms-btn--danger'))).toHaveLength(1)
    await typePassword(m, 'correct horse')
    expect(confirm().props.disabled).toBe(false)
  })

  test('confirming turns the password into a token first, then deletes with THAT token; the row leaves and the sheet closes', async () => {
    const { m, modal } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'correct horse')
    await press(m, 'Delete permanently')
    expect(calls(m, 'desktop_confirm_action')).toEqual([{ password: 'correct horse' }])
    expect(calls(m, 'desktop_trash_delete_permanently')).toEqual([{ fileId: 't1', confirmToken: 'tok-1' }])
    expect(m.calls.map((c) => c.name).filter((n) => n.startsWith('desktop_'))).toEqual([
      'desktop_trash_list', 'desktop_confirm_action', 'desktop_trash_delete_permanently',
    ])
    expect(rowsOf(m)).toHaveLength(1)
    expect(modalNow(m, modal).open).toBe(false)
  })

  test('a wrong password keeps the sheet, says why once, never calls delete, and the password is still there to correct', async () => {
    const { m, modal } = open({ desktop_confirm_action: () => { throw new Error('Password is incorrect') } })
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'nope')
    await press(m, 'Delete permanently')
    expect(calls(m, 'desktop_trash_delete_permanently')).toHaveLength(0)
    expect(modalNow(m, modal).open).toBe(true)
    const alerts = find(m, (el) => el.props.role === 'alert')
    expect(alerts).toHaveLength(1)
    expect(lines(m).join(' | ')).toContain('Password is incorrect')
    expect(m.toasts).toEqual([])
    expect(rowsOf(m)).toHaveLength(2)
  })

  test('a delete the server refuses keeps the sheet and the row, and says why', async () => {
    const { m } = open({ desktop_trash_delete_permanently: () => { throw new Error('file is shared') } })
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'correct horse')
    await press(m, 'Delete permanently')
    expect(lines(m).join(' | ')).toContain('file is shared')
    expect(rowsOf(m)).toHaveLength(2)
  })

  test('Cancel and the close button forget the password and the error; reopening starts clean', async () => {
    const { m, modal } = open({ desktop_confirm_action: () => { throw new Error('Password is incorrect') } })
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'nope')
    await press(m, 'Delete permanently')
    await press(m, 'Cancel')
    expect(modalNow(m, modal).open).toBe(false)
    await settle(m)
    await pressFirst(m, 'Delete…')
    expect(password(m).props.value).toBe('')
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(0)
  })

  test('before the delete starts, Esc and the close button close the sheet', async () => {
    const { m, modal } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    modalNow(m, modal).onClose()
    m.render()
    expect(modalNow(m, modal).open).toBe(false)
    expect(calls(m, 'desktop_confirm_action')).toHaveLength(0)
  })

  test('while the delete runs nothing closes the sheet (the outcome must be seen)', async () => {
    let release: () => void = () => {}
    const { m, modal } = open({ desktop_trash_delete_permanently: () => new Promise<null>((resolve) => { release = () => resolve(null) }) })
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'correct horse')
    await m.clickNoWait('Delete permanently')
    await settle(m)
    expect(button(m, 'Deleting…').props.disabled).toBe(true)
    expect(button(m, 'Cancel').props.disabled).toBe(true)
    modalNow(m, modal).onClose()
    m.render()
    expect(modalNow(m, modal).open).toBe(true)
    release()
    await settle(m)
    expect(modalNow(m, modal).open).toBe(false)
  })

  test('Enter in the filled password field confirms (a deliberate act after typing); on open, focus is the close button, so Enter alone cancels', async () => {
    const { m } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'correct horse')
    let prevented = false
    password(m).props.onKeyDown({ key: 'Enter', preventDefault: () => { prevented = true } })
    await settle(m)
    expect(prevented).toBe(true)
    expect(calls(m, 'desktop_trash_delete_permanently')).toHaveLength(1)
  })

  test('Enter in an empty password field does nothing', async () => {
    const { m } = open()
    await settle(m)
    await pressFirst(m, 'Delete…')
    password(m).props.onKeyDown({ key: 'Enter', preventDefault() {} })
    await settle(m)
    expect(calls(m, 'desktop_confirm_action')).toHaveLength(0)
  })
})

describe('Refresh (the toolbar button lives in the window; it reaches the pane as a number)', () => {
  test('a new refresh signal loads again, and the same one does not', async () => {
    const { m, props } = open({}, 0)
    await settle(m)
    expect(calls(m, 'desktop_trash_list')).toHaveLength(1)
    m.render()
    await settle(m)
    expect(calls(m, 'desktop_trash_list')).toHaveLength(1)
    props.refreshSignal = 1
    m.render()
    await settle(m)
    expect(calls(m, 'desktop_trash_list')).toHaveLength(2)
  })

  test('an answer to an older load never lands over a newer one', async () => {
    const order: Array<(v: unknown) => void> = []
    const { m, props } = open({ desktop_trash_list: () => new Promise((resolve) => { order.push(resolve) }) }, 0)
    await settle(m)
    props.refreshSignal = 1
    m.render()
    await settle(m)
    expect(order).toHaveLength(2)
    order[1]([{ id: 'new', name: 'Newer.txt', is_folder: false, size_bytes: 1, updated_at: '' }])
    await settle(m)
    order[0]([{ id: 'old', name: 'Older.txt', is_folder: false, size_bytes: 1, updated_at: '' }])
    await settle(m)
    expect(lines(m).join(' | ')).toContain('Newer.txt')
    expect(lines(m).join(' | ')).not.toContain('Older.txt')
  })

  test('a refresh still pending never puts back a row a Restore or a delete has removed since (review thread on #142)', async () => {
    const lists: Array<(v: unknown) => void> = []
    let first = true
    const { m, props } = open({
      desktop_trash_list: () => {
        if (first) { first = false; return files() }
        return new Promise((resolve) => { lists.push(resolve) })
      },
    }, 0)
    await settle(m)
    props.refreshSignal = 1 // Refresh: the list is asked for again and the old rows stay while it is out
    m.render()
    await settle(m)
    expect(lists).toHaveLength(1)
    await pressFirst(m, 'Restore') // t1 is restored while the refresh is still out
    expect(rowsOf(m)).toHaveLength(1)
    lists[0](files()) // the refresh answers with the list as it was before the restore
    await settle(m)
    expect(lines(m).join(' | ')).not.toContain('Draft 3')
    expect(rowsOf(m)).toHaveLength(1)
  })

  test('the same for a permanent delete', async () => {
    const lists: Array<(v: unknown) => void> = []
    let first = true
    const { m, props } = open({
      desktop_trash_list: () => {
        if (first) { first = false; return files() }
        return new Promise((resolve) => { lists.push(resolve) })
      },
    }, 0)
    await settle(m)
    props.refreshSignal = 1
    m.render()
    await settle(m)
    await pressFirst(m, 'Delete…')
    await typePassword(m, 'correct horse')
    await press(m, 'Delete permanently')
    expect(rowsOf(m)).toHaveLength(1)
    lists[0](files())
    await settle(m)
    expect(rowsOf(m)).toHaveLength(1)
    expect(lines(m).join(' | ')).not.toContain('Draft 3')
  })

  test('it exposes no Refresh button of its own: one place for it (the toolbar)', async () => {
    const { m } = open()
    await settle(m)
    expect(() => button(m, 'Refresh')).toThrow()
    void buttonLabel
  })
})
