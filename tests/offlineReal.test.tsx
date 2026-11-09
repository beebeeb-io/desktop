// Available offline lists what drive_offline_list returned. An empty list is not a failed read.
// A folder's nested count and bytes are that command's numbers, and only the top-most kept items are shown.
// Remove waits for set_recursive_pin: a failure leaves the row, and Remove all says how many the command changed.
import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import type { DriveItem } from '@beebeeb/shared/drive-ui'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const READ_ERROR = 'The offline list could not be read.'
const PIN_ERROR = 'The copy is in use.'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let Section: typeof import('../src/v2/sections/available-offline').default
let DriveSourceProvider: typeof import('@beebeeb/shared/drive-ui').DriveSourceProvider
let ToastHost: typeof import('../src/v2/chrome/Toast').ToastHost

type Entry = { item: DriveItem; items?: number; bytes: number }
type Listener = (event: { kind: string; item?: DriveItem }) => void
type Gate = { resolve: (value: unknown) => void; reject: (error: unknown) => void }

let list: Entry[] = []
let failRead = false
const listeners: Listener[] = []
const pending: Gate[] = []

function item(over: Partial<DriveItem> & Pick<DriveItem, 'id' | 'name' | 'isFolder'>): DriveItem {
  return {
    parentId: null, size: null, modifiedAt: '2026-11-01T10:00:00', shared: { kind: 'none' },
    local: 'available-offline', hasThumbnail: false, ...over,
  }
}

function hold(): Promise<unknown> {
  return new Promise((resolve, reject) => { pending.push({ resolve, reject }) })
}

beforeAll(async () => {
  dom = installMiniDom('')
  const heard = new Map<string, Set<(event: Event) => void>>()
  const add = dom.window.addEventListener.bind(dom.window)
  const remove = dom.window.removeEventListener.bind(dom.window)
  dom.window.addEventListener = (type: string, fn: (event: Event) => void) => {
    const set = heard.get(type) ?? new Set()
    set.add(fn)
    heard.set(type, set)
    add(type, fn)
  }
  dom.window.removeEventListener = (type: string, fn: (event: Event) => void) => {
    heard.get(type)?.delete(fn)
    remove(type, fn)
  }
  dom.window.dispatchEvent = (event: Event) => {
    for (const fn of heard.get(event.type) ?? []) fn(event)
    return true
  }
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string) {
      if (cmd === 'drive_offline_list') {
        return failRead ? Promise.reject(new Error(READ_ERROR)) : Promise.resolve(list)
      }
      if (cmd === 'set_recursive_pin') return hold()
      return Promise.reject(new Error(`unscripted ${cmd}`))
    },
  }
  React = await import('react')
  client = await import('react-dom/client')
  DriveSourceProvider = (await import('@beebeeb/shared/drive-ui')).DriveSourceProvider
  ToastHost = (await import('../src/v2/chrome/Toast')).ToastHost
  Section = (await import('../src/v2/sections/available-offline')).default
})
afterAll(() => dom.restore())

function source() {
  return {
    subscribe(fn: Listener) {
      listeners.push(fn)
      return () => { const at = listeners.indexOf(fn); if (at >= 0) listeners.splice(at, 1) }
    },
    setAvailableOffline: () => hold(),
    open: async () => {},
    revealInFileManager: async () => {},
  }
}

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

function button(label: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === label)
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled ${label}`)
  return found[0]!
}

function row(id: string): MiniElement | undefined {
  return allElements(dom.document.body).find((el) => el.getAttribute('data-row') === id)
}

async function show() {
  clearBody()
  listeners.length = 0
  pending.length = 0
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  const tree = React.createElement(ToastHost, null,
    React.createElement(DriveSourceProvider, { source: source() as never }, React.createElement(Section, { nav: {} as never })))
  await React.act(async () => { root.render(tree) })
  await settle()
  return {
    container: container as MiniElement,
    text: () => dom.document.body.textContent ?? '',
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

describe.serial('Available offline lists what is kept on this PC', () => {
  test('an empty list says nothing is kept, and a failed read is not that empty list', async () => {
    failRead = true
    list = []
    const view = await show()
    expect(view.text()).toContain(READ_ERROR)
    expect(view.text()).toContain('Try again')
    expect(view.text()).not.toContain('Nothing is kept on this PC')
    expect(view.text()).not.toContain('Nothing available offline')
    failRead = false
    await React.act(async () => { click(view.container, button('Try again')) })
    await settle()
    expect(view.text()).toContain('Nothing is kept on this PC')
    expect(view.text()).not.toContain(READ_ERROR)
    expect(view.text()).not.toContain('Nothing available offline')
    expect(view.text()).not.toContain('Try again')
    const link = allElements(dom.document.body).find((el) => el.tagName === 'A' && (el.getAttribute('href') ?? '').includes('section=files'))
    expect(link?.textContent ?? '').toContain('Files')
    await view.unmount()
  })

  test('a folder shows the command’s own nested count and bytes, and only the top-most kept items', async () => {
    failRead = false
    const folder = item({ id: 'folder', name: 'Field notes', isFolder: true, childCount: 99, local: 'syncing' })
    const nested = item({ id: 'child', parentId: 'folder', name: 'nested-a.pdf', isFolder: false, size: 100 })
    const loose = item({ id: 'loose', name: 'Loose.txt', isFolder: false, size: 99999 })
    list = [
      { item: folder, items: 7, bytes: 2_500_000 },
      { item: nested, bytes: 100 },
      { item: loose, bytes: 84 },
    ]
    const view = await show()
    expect(row('folder')?.textContent ?? '').toContain('Field notes')
    expect(row('folder')?.textContent ?? '').toContain('7 items')
    expect(row('folder')?.textContent ?? '').toContain('2.5 MB')
    expect(row('folder')?.textContent ?? '').toContain('Downloading')
    expect(row('loose')?.textContent ?? '').toContain('Loose.txt')
    expect(row('loose')?.textContent ?? '').toContain('84 B')
    expect(row('loose')?.textContent ?? '').toContain('Up to date')
    expect(row('child')).toBeUndefined()
    expect(view.text()).not.toContain('nested-a.pdf')
    expect(view.text()).not.toContain('99 items')
    expect(view.text()).not.toContain('100 KB')
    expect(view.text()).not.toContain('100 B')
    expect(view.text()).not.toContain('0 items')
    await React.act(async () => {
      for (const fn of [...listeners]) fn({ kind: 'item', item: { ...folder, local: 'available-offline' } })
    })
    await settle()
    expect(row('folder')?.textContent ?? '').toContain('Up to date')
    expect(row('folder')?.textContent ?? '').not.toContain('Downloading')
    expect(row('folder')?.textContent ?? '').toContain('7 items')
    expect(row('folder')?.textContent ?? '').toContain('2.5 MB')
    expect(row('child')).toBeUndefined()
    await view.unmount()
  })

  test('a failed removal leaves the row, and remove all reports how many the command removed', async () => {
    failRead = false
    list = [{ item: item({ id: 'notes', name: 'Notes.txt', isFolder: false, size: 10 }), bytes: 10 }]
    const view = await show()
    expect(row('notes')?.textContent ?? '').toContain('Notes.txt')
    await React.act(async () => { click(view.container, button('Remove all from this PC')) })
    await React.act(async () => { click(view.container, button('Remove all')) })
    expect(pending).toHaveLength(1)
    expect(row('notes')).toBeTruthy()
    expect(view.text()).not.toContain('Removed')
    await React.act(async () => { pending.shift()!.reject(new Error(PIN_ERROR)) })
    await settle()
    expect(row('notes')?.textContent ?? '').toContain('Notes.txt')
    expect(view.text()).toContain(PIN_ERROR)
    expect(view.text()).not.toContain('Removed')
    expect(view.text()).not.toContain('Nothing is kept on this PC')
    expect(view.text()).not.toContain('Nothing available offline')
    await React.act(async () => { click(view.container, button('Remove all from this PC')) })
    await React.act(async () => { click(view.container, button('Remove all')) })
    expect(pending).toHaveLength(1)
    expect(row('notes')).toBeTruthy()
    await React.act(async () => { pending.shift()!.resolve({ changed_item_ids: ['notes', 'notes-copy'] }) })
    await settle()
    expect(row('notes')).toBeUndefined()
    expect(view.text()).toContain('Removed 2 from this PC.')
    expect(view.text()).not.toContain('Removed 1')
    expect(view.text()).not.toContain('Notes.txt')
    expect(view.text()).toContain('Nothing is kept on this PC')
    await view.unmount()
  })
})
