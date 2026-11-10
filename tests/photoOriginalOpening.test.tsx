// DW-0605b, live route: choosing Original starts the source's open and the viewer says it is opening until that read
// settles. A refusal uses the failed toast and stays on the preview, never a mode that claims the original is there.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { DriveSourceProvider, type DriveItem, type DriveSource, type PhotoEntry } from '@beebeeb/shared/drive-ui'
import { ToastHost } from '../src/v2/chrome/Toast'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let PhotosSection: typeof import('../src/v2/sections/photos').default

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  const proto = dom.window.HTMLElement.prototype as { querySelector: () => null; querySelectorAll: () => [] }
  proto.querySelector = () => null
  proto.querySelectorAll = () => []
  if (typeof globalThis.requestAnimationFrame !== 'function') {
    globalThis.requestAnimationFrame = (fn: FrameRequestCallback) => setTimeout(() => fn(Date.now()), 0) as unknown as number
    globalThis.cancelAnimationFrame = (id: number) => clearTimeout(id as unknown as ReturnType<typeof setTimeout>)
  }
  const store = new Map<string, string>()
  dom.window.localStorage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => { store.set(key, value) },
    removeItem: (key: string) => { store.delete(key) },
  }
  const listeners = new Map<string, Array<(event: Event) => void>>()
  const add = dom.window.addEventListener.bind(dom.window)
  dom.window.addEventListener = (type: string, fn: (event: Event) => void, options?: unknown) => {
    const list = listeners.get(type) ?? []
    list.push(fn)
    listeners.set(type, list)
    add(type, fn, options)
  }
  dom.window.dispatchEvent = (event: Event) => {
    for (const fn of listeners.get(event.type) ?? []) fn(event)
    return true
  }
  PhotosSection = (await import('../src/v2/sections/photos')).default
})
afterAll(() => dom.restore())

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

async function settle() {
  for (let i = 0; i < 10; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

function host(answer: (cmd: string, args?: Record<string, unknown>) => unknown) {
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) { return Promise.resolve(answer(cmd, args)) },
  }
  // The transfer subscription registers through Tauri's event plugin; the mini DOM has no plugin host of its own.
  ;(dom.window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    registerListener() { return 1 },
    unregisterListener() {},
  }
}

function entry(id: string, name: string, takenAt: string, size: number): PhotoEntry {
  const file: DriveItem = {
    id, parentId: 'folder-photos', name, isFolder: false, size, modifiedAt: takenAt, mime: 'image/jpeg',
    shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: true,
  }
  return { item: file, takenAt }
}

function sourceOf(entries: PhotoEntry[], open: (id: string) => Promise<void>): DriveSource {
  return {
    kind: 'desktop',
    capabilities: { localCopies: true, revealInFileManager: true, office: false, nativePickers: false },
    photos: async () => ({ entries }),
    thumbnail: async (id: string) => `mem://${id}`,
    subscribe() { return () => {} },
    open,
  } as unknown as DriveSource
}

async function show(source: DriveSource) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(<DriveSourceProvider source={source}><ToastHost><PhotosSection nav={{ section: 'photos' } as never} /></ToastHost></DriveSourceProvider>) })
  await settle()
  return {
    container: container as MiniElement,
    text: () => (dom.document.body.textContent ?? '').replace(/\s+/g, ' ').trim(),
    async unmount() { await React.act(async () => { root.unmount() }); container.parentNode?.removeChild(container) },
  }
}

function originalButton(): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').replace(/\s+/g, ' ').trim().startsWith('Original'))
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled Original`)
  return found[0]!
}

function pressed(label: string): string | null {
  const found = allElements(dom.document.body).find((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').replace(/\s+/g, ' ').trim().startsWith(label))
  return found?.getAttribute('aria-pressed') ?? null
}

async function openViewer() {
  const tile = allElements(dom.document.body).find((el) => el.getAttribute('data-tile') === 'opening-one')
  if (!tile) throw new Error('no photo tile')
  await React.act(async () => { click(dom.document.body.firstChild as MiniElement, tile) })
  await settle()
}

test('choosing Original while the read is in flight says it is opening, and the status leaves when it settles', async () => {
  clearBody()
  host((cmd) => (cmd === 'drive_photo_meta' ? {} : cmd === 'drive_photo_sources' ? [] : cmd === 'drive_transfers' ? [] : null))
  let finish: (() => void) | null = null
  const opened: string[] = []
  const source = sourceOf([entry('opening-one', 'Harbour wall.jpg', '2026-04-03T10:00:00', 30_900_000)], (id) => {
    opened.push(id)
    return new Promise<void>((resolve) => { finish = resolve })
  })
  const pane = await show(source)
  await openViewer()
  await React.act(async () => { click(pane.container, originalButton()) })
  await settle()
  const whileOpening = pane.text()
  expect({
    opened,
    status: whileOpening.includes('Opening the original…'),
    original: pressed('Original'),
    preview: pressed('Preview'),
  }).toEqual({ opened: ['opening-one'], status: true, original: 'true', preview: 'false' })
  await React.act(async () => { finish!() })
  await settle()
  const settled = pane.text()
  expect({
    status: settled.includes('Opening the original…'),
    original: pressed('Original'),
    preview: pressed('Preview'),
  }).toEqual({ status: false, original: 'true', preview: 'false' })
  await pane.unmount()
})

test('a refused open says so and leaves the viewer on the preview', async () => {
  clearBody()
  host((cmd) => (cmd === 'drive_photo_meta' ? {} : cmd === 'drive_photo_sources' ? [] : cmd === 'drive_transfers' ? [] : null))
  const opened: string[] = []
  const source = sourceOf([entry('opening-one', 'Harbour wall.jpg', '2026-04-03T10:00:00', 30_900_000)], async (id) => {
    opened.push(id)
    throw new Error('the original is not reachable')
  })
  const pane = await show(source)
  await openViewer()
  await React.act(async () => { click(pane.container, originalButton()) })
  await settle()
  const text = pane.text()
  expect({
    opened,
    toast: text.includes('Couldn’t open the original. Try again.'),
    status: text.includes('Opening the original…'),
    original: pressed('Original'),
    preview: pressed('Preview'),
  }).toEqual({
    opened: ['opening-one'], toast: true, status: false, original: 'false', preview: 'true',
  })
  await pane.unmount()
})
