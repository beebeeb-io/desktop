// Keep on this PC must draw the vault's own folders. The tree, the checked set and the plan come from
// `list_vault_folders` and `get_selective_sync`; Apply writes that selection and shows the command's answer.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { VaultItem } from '../src/desktopApi'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let keep: typeof import('../src/v2/sections/settings/today/keep')
let Ctx: typeof import('../src/v2/sections/settings/shell/context').Ctx

beforeAll(async () => {
  dom = installMiniDom('')
  const proto = dom.window.HTMLElement.prototype as { querySelector: (selector: string) => MiniElement | null }
  proto.querySelector = function (this: MiniElement, selector: string) {
    return allElements(this).find((el) => el !== this && (
      (selector.includes('button') && el.tagName === 'BUTTON') || (selector.includes('input') && el.tagName === 'INPUT')
    )) ?? null
  }
  React = await import('react')
  client = await import('react-dom/client')
  keep = await import('../src/v2/sections/settings/today/keep')
  Ctx = (await import('../src/v2/sections/settings/shell/context')).Ctx
})
afterAll(() => dom.restore())

function item(over: Partial<VaultItem> & Pick<VaultItem, 'id' | 'name'>): VaultItem {
  const node: VaultItem = { is_folder: true, excluded: false, size_bytes: 0, file_count: 0, on_disk_bytes: 0, children: [], ...over }
  if (!Object.prototype.hasOwnProperty.call(over, 'size_bytes')) delete (node as { size_bytes?: number }).size_bytes
  if (!Object.prototype.hasOwnProperty.call(over, 'file_count')) delete (node as { file_count?: number }).file_count
  return node
}

function counted(value: object): { files: number | undefined; bytes: number | undefined } {
  const row = value as { files?: number; bytes?: number }
  return { files: row.files, bytes: row.bytes }
}

const elements = (container: MiniElement) => allElements(container)
function byText(container: MiniElement, tag: string, text: string): MiniElement {
  const hits = elements(container).filter((el) => el.tagName === tag && el.textContent.trim() === text)
  if (hits.length !== 1) throw new Error(`expected one ${tag} "${text}", found ${hits.length}`)
  return hits[0]!
}
function byLabel(container: MiniElement, label: string): MiniElement {
  const hits = elements(container).filter((el) => el.getAttribute('aria-label') === label)
  if (hits.length !== 1) throw new Error(`expected one control "${label}", found ${hits.length}`)
  return hits[0]!
}
const checkedLabels = (container: MiniElement) => elements(container)
  .filter((el) => el.getAttribute('role') === 'checkbox' && el.getAttribute('aria-checked') === 'true')
  .map((el) => el.getAttribute('aria-label'))

interface Script {
  folders: VaultItem[]
  excluded: string[]
  failRead?: boolean
  failWrite?: string | null
}

function script(next: Script) {
  const calls: { name: string; args: Record<string, unknown> | undefined }[] = []
  let failWrite = next.failWrite ?? null
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(name: string, args?: Record<string, unknown>) {
      calls.push({ name, args })
      if (name === 'list_vault_folders') {
        return next.failRead ? Promise.reject(new Error('could not list folders')) : Promise.resolve(next.folders)
      }
      if (name === 'get_selective_sync') {
        return next.failRead ? Promise.reject(new Error('could not read which folders stay on this PC')) : Promise.resolve(next.excluded)
      }
      if (name === 'set_selective_sync') {
        // A success answers on a later turn, so a close that runs before the answer is visible.
        return failWrite ? Promise.reject(new Error(failWrite)) : new Promise((resolve) => { setTimeout(() => resolve(null), 0) })
      }
      return Promise.reject(new Error(`unscripted ${name}`))
    },
  }
  return {
    calls,
    setFailWrite(message: string | null) { failWrite = message },
    written(): string[] {
      const saves = calls.filter((call) => call.name === 'set_selective_sync')
      const last = saves[saves.length - 1]
      const ids = last?.args?.excluded
      return Array.isArray(ids) ? ids.map(String) : []
    },
  }
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)) })
}

function mount(node: import('react').ReactElement) {
  const container = dom.document.createElement('div') as MiniElement
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  return { container, root, show: async (next: import('react').ReactElement) => { await React.act(async () => { root.render(next) }); await settle() } }
}

describe.serial('Keep on this PC shows the vault’s own folders', () => {
  const opened: string[] = []
  let closed = 0
  const settings = () => ({ pane: 'sync' as const, go() {}, dialog: 'keep', open(id: string) { opened.push(id) }, close() { closed += 1 } })

  test('the tree and the plan come only from list_vault_folders and get_selective_sync', async () => {
    const folders = [
      item({
        id: 'work', name: 'Field notes', size_bytes: 5000, file_count: 9,
        children: [item({ id: 'nested', name: 'Nested', size_bytes: 1000, file_count: 2 })],
      }),
      item({ id: 'loose', name: 'Loose scans' }),
    ]
    const api = script({ folders, excluded: [] })
    const view = mount(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    await view.show(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    const { tree, checked } = keep.keepTreeFrom(folders, [])
    const plan = counted(keep.plan({ checked, initial: new Set(checked) }))
    expect(api.calls.map((call) => call.name)).toEqual(['list_vault_folders', 'get_selective_sync'])
    expect(tree.map((node) => node.name)).toEqual(['Field notes', 'Loose scans'])
    expect(tree[0]?.children?.map((node) => node.name)).toEqual(['Nested'])
    expect([...checked].sort()).toEqual(['loose', 'work'])
    expect(plan.files).toBe(9)
    expect(plan.bytes).toBe(5000)
    expect(view.container.textContent).toContain('Field notes')
    expect(view.container.textContent).toContain('Loose scans')
    expect(view.container.textContent).toContain('9 files')
    expect(view.container.textContent).toContain('5 KB')
    expect(view.container.textContent).not.toContain('NaN')
    expect(view.container.textContent).not.toContain('0 MB')
    expect(view.container.textContent).not.toContain('Lisbon')
    expect(view.container.textContent).not.toContain('Invoices 2026')
    const loose = elements(view.container).find((el) => el.getAttribute('data-testid') === 'keep-row' && el.textContent.includes('Loose scans'))
    expect(loose?.textContent ?? '').not.toMatch(/\d/)
    await React.act(async () => view.root.unmount())
  })

  test('an empty vault shows the empty state', async () => {
    const api = script({ folders: [], excluded: [] })
    const view = mount(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    await view.show(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    expect(api.calls.map((call) => call.name)).toEqual(['list_vault_folders', 'get_selective_sync'])
    expect(view.container.textContent).toContain('No folders yet')
    expect(view.container.textContent).not.toContain('Field notes')
    expect(view.container.textContent).not.toContain('Lisbon')
    expect(elements(view.container).some((el) => el.getAttribute('data-testid') === 'keep-row')).toBe(false)
    await React.act(async () => view.root.unmount())
  })

  test('Apply writes exactly the selection and a failed write leaves it on screen', async () => {
    const folders = [
      item({
        id: 'projects', name: 'Projects', size_bytes: 8000, file_count: 4,
        children: [
          item({ id: 'alpha', name: 'Alpha', size_bytes: 3000, file_count: 1 }),
          item({ id: 'beta', name: 'Beta', size_bytes: 5000, file_count: 3 }),
        ],
      }),
    ]
    const api = script({ folders, excluded: [], failWrite: 'could not save the folder choice' })
    opened.length = 0
    closed = 0
    const ctx = settings()
    const view = mount(React.createElement(Ctx.Provider, { value: ctx }, React.createElement(keep.KeepDialog)))
    await view.show(React.createElement(Ctx.Provider, { value: ctx }, React.createElement(keep.KeepDialog)))
    expect(byLabel(view.container, 'Projects').getAttribute('aria-checked')).toBe('true')
    await React.act(async () => { click(view.container, byLabel(view.container, 'Projects')) })
    expect(byLabel(view.container, 'Projects').getAttribute('aria-checked')).toBe('false')
    await React.act(async () => { click(view.container, byText(view.container, 'BUTTON', 'Apply')) })
    let confirm = view
    if (opened.includes('keep-confirm')) {
      confirm = mount(React.createElement(Ctx.Provider, { value: ctx }, React.createElement(keep.KeepConfirm)))
      await confirm.show(React.createElement(Ctx.Provider, { value: ctx }, React.createElement(keep.KeepConfirm)))
      await React.act(async () => { click(confirm.container, byText(confirm.container, 'BUTTON', 'Apply')) })
    }
    await settle()
    expect(api.written().slice().sort()).toEqual(['projects'])
    expect(`${view.container.textContent} ${confirm.container.textContent}`).toContain('could not save the folder choice')
    expect(closed).toBe(0)
    expect(byLabel(view.container, 'Projects').getAttribute('aria-checked')).toBe('false')
    expect(checkedLabels(view.container).filter((label) => label === 'Projects' || label === 'Alpha' || label === 'Beta')).toEqual([])

    api.setFailWrite(null)
    await React.act(async () => { click(confirm.container, byText(confirm.container, 'BUTTON', 'Apply')) })
    expect(closed).toBe(0)
    await settle()
    expect(api.written().slice().sort()).toEqual(['projects'])
    expect(closed).toBe(1)
    expect(`${view.container.textContent} ${confirm.container.textContent}`).not.toContain('could not save the folder choice')
    await React.act(async () => view.root.unmount())
    if (confirm !== view) await React.act(async () => confirm.root.unmount())
  })

  test('a folder the read says is not kept stays unchecked, and a failed read does not select everything', async () => {
    const folders = [
      item({ id: 'alpha', name: 'Alpha', excluded: true, size_bytes: 100, file_count: 1 }),
      item({ id: 'beta', name: 'Beta', excluded: false, size_bytes: 200, file_count: 2 }),
    ]
    const { checked } = keep.keepTreeFrom(folders, [])
    expect(checked.has('alpha')).toBe(false)
    expect(checked.has('beta')).toBe(true)
    const listed = keep.keepTreeFrom([item({ id: 'beta', name: 'Beta', excluded: false, size_bytes: 200, file_count: 2 })], ['beta'])
    expect(listed.checked.has('beta')).toBe(false)

    const shown = script({ folders, excluded: [] })
    const view = mount(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    await view.show(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    expect(shown.calls.map((call) => call.name)).toEqual(['list_vault_folders', 'get_selective_sync'])
    expect(counted(keep.plan({ checked, initial: new Set(checked) })).bytes).toBe(200)
    expect(counted(keep.plan({ checked, initial: new Set(checked) })).files).toBe(2)
    expect(checkedLabels(view.container)).toContain('Beta')
    expect(checkedLabels(view.container)).not.toContain('Alpha')
    await React.act(async () => view.root.unmount())

    script({ folders, excluded: [], failRead: true })
    const failed = mount(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    await failed.show(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(keep.KeepDialog)))
    expect(failed.container.textContent).toContain("Couldn't load your folders.")
    expect(checkedLabels(failed.container)).toEqual([])
    keep.resetKeep()
    await settle()
    expect(checkedLabels(failed.container)).toEqual([])
    expect(failed.container.textContent).not.toContain('Alpha')
    expect(failed.container.textContent).not.toContain('Beta')
    await React.act(async () => failed.root.unmount())
  })
})
