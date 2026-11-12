// Slice 01b: the two save-a-copy buttons call drive_save_copy { id } and say what happened honestly. A true answer
// is "Saved a copy"; a cancelled dialog (false) says nothing; a refusal uses actionFailure's words, never the host's.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { DriveSourceProvider, type DriveItem, type DriveSource, type PhotoEntry } from '@beebeeb/shared/drive-ui'
import { ToastHost } from '../src/v2/chrome/Toast'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let OpenSection: typeof import('../src/v2/sections/open').default
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
  OpenSection = (await import('../src/v2/sections/open')).default
  PhotosSection = (await import('../src/v2/sections/photos')).default
})
afterAll(() => dom.restore())

type Call = { cmd: string; args?: Record<string, unknown> }

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
  ;(dom.window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    registerListener() { return 1 },
    unregisterListener() {},
  }
}

function button(label: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').replace(/\s+/g, ' ').trim() === label)
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled ${label}`)
  return found[0]!
}

function bigItem(): DriveItem {
  return {
    id: 'too-large-file', parentId: 'folder-docs', name: 'Holiday video.mp4', isFolder: false, size: 4_000_000_000,
    modifiedAt: '2026-04-03T10:00:00', mime: 'video/mp4', shared: { kind: 'none' }, local: 'online-only', hasThumbnail: false,
  }
}

function openSource(item: DriveItem): DriveSource {
  return {
    kind: 'desktop',
    capabilities: { localCopies: true, revealInFileManager: true, office: false, nativePickers: false },
    details: async () => ({ item, path: [], versionCount: 1 }),
    subscribe() { return () => {} },
  } as unknown as DriveSource
}

async function showOpen(item: DriveItem) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  const nav = { route: { section: 'open', id: item.id, sub: null }, go() {}, back() {}, forward() {} }
  await React.act(async () => {
    root.render(
      <DriveSourceProvider source={openSource(item)}>
        <ToastHost><OpenSection nav={nav as never} /></ToastHost>
      </DriveSourceProvider>,
    )
  })
  await settle()
  return {
    text: () => (dom.document.body.textContent ?? '').replace(/\s+/g, ' ').trim(),
    async unmount() { await React.act(async () => { root.unmount() }); container.parentNode?.removeChild(container) },
  }
}

function photoEntry(id: string): PhotoEntry {
  return {
    item: {
      id, parentId: 'folder-photos', name: 'Harbour wall.jpg', isFolder: false, size: 4_000_000,
      modifiedAt: '2026-04-03T10:00:00', mime: 'image/jpeg', shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: true,
    },
    takenAt: '2026-04-03T10:00:00',
  }
}

function photoSource(entry: PhotoEntry): DriveSource {
  return {
    kind: 'desktop',
    capabilities: { localCopies: true, revealInFileManager: true, office: false, nativePickers: false },
    photos: async () => ({ entries: [entry] }),
    thumbnail: async (id: string) => `mem://${id}`,
    subscribe() { return () => {} },
  } as unknown as DriveSource
}

async function showPhotos(entry: PhotoEntry) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => {
    root.render(
      <DriveSourceProvider source={photoSource(entry)}>
        <ToastHost><PhotosSection nav={{ section: 'photos' } as never} /></ToastHost>
      </DriveSourceProvider>,
    )
  })
  await settle()
  return {
    container: container as MiniElement,
    text: () => (dom.document.body.textContent ?? '').replace(/\s+/g, ' ').trim(),
    async unmount() { await React.act(async () => { root.unmount() }); container.parentNode?.removeChild(container) },
  }
}

test('too_large_download_to_calls_the_command', async () => {
  clearBody()
  const calls: Call[] = []
  host((cmd, args) => {
    calls.push({ cmd, args })
    if (cmd === 'drive_status') return { freeSpaceBytes: 10, backup: { state: 'off', foldersOn: 0, foldersGettingReady: 0 }, signedOut: false, stoppedFiles: 0, syncProblems: 0, backupProblems: 0, conflicts: [] }
    if (cmd === 'drive_save_copy') return true
    return null
  })
  const pane = await showOpen(bigItem())
  await React.act(async () => { click(dom.document.body.firstChild as MiniElement, button('Download to…')) })
  await settle()
  const text = pane.text()
  expect({
    called: calls.filter((c) => c.cmd === 'drive_save_copy').map((c) => c.args),
    oldSentence: text.includes('decrypted copy elsewhere'),
    unavailable: text.includes('isn’t available in this version'),
  }).toEqual({ called: [{ id: 'too-large-file' }], oldSentence: false, unavailable: false })
  await pane.unmount()
})

test('a_saved_copy_says_so', async () => {
  clearBody()
  const answers = [true, false]
  let answer: boolean | null = null
  host((cmd) => {
    if (cmd === 'drive_status') return { freeSpaceBytes: 10, backup: { state: 'off', foldersOn: 0, foldersGettingReady: 0 }, signedOut: false, stoppedFiles: 0, syncProblems: 0, backupProblems: 0, conflicts: [] }
    if (cmd === 'drive_save_copy') return answer
    return null
  })
  const seen: { answer: boolean | null; saved: boolean; failed: boolean }[] = []
  for (const next of answers) {
    clearBody()
    answer = next
    const pane = await showOpen(bigItem())
    await React.act(async () => { click(dom.document.body.firstChild as MiniElement, button('Download to…')) })
    await settle()
    const text = pane.text()
    seen.push({ answer: next, saved: text.includes('Saved a copy'), failed: /Couldn’t/.test(text) })
    await pane.unmount()
  }
  expect(seen).toEqual([{ answer: true, saved: true, failed: false }, { answer: false, saved: false, failed: false }])
})

test('a_refused_copy_says_why', async () => {
  clearBody()
  host((cmd) => {
    if (cmd === 'drive_status') return { freeSpaceBytes: 10, backup: { state: 'off', foldersOn: 0, foldersGettingReady: 0 }, signedOut: false, stoppedFiles: 0, syncProblems: 0, backupProblems: 0, conflicts: [] }
    if (cmd === 'drive_save_copy') throw new Error('host detail: ENOSPC at C:\\Users\\guus')
    return null
  })
  const pane = await showOpen(bigItem())
  await React.act(async () => { click(dom.document.body.firstChild as MiniElement, button('Download to…')) })
  await settle()
  const text = pane.text()
  expect({
    failure: text.includes('Couldn’t save a copy to this PC.'),
    hostText: text.includes('ENOSPC') || text.includes('C:\\Users'),
    saved: text.includes('Saved a copy'),
  }).toEqual({ failure: true, hostText: false, saved: false })
  await pane.unmount()
})

test('photo_save_to_pc_calls_the_command', async () => {
  clearBody()
  const calls: Call[] = []
  host((cmd, args) => {
    calls.push({ cmd, args })
    if (cmd === 'drive_photo_meta') return {}
    if (cmd === 'drive_photo_sources') return []
    if (cmd === 'drive_transfers') return []
    if (cmd === 'drive_save_copy') return true
    return null
  })
  const pane = await showPhotos(photoEntry('photo-1'))
  const tile = allElements(dom.document.body).find((el) => el.getAttribute('data-tile') === 'photo-1')
  if (!tile) throw new Error('no photo tile')
  await React.act(async () => { click(pane.container, tile) })
  await settle()
  const more = allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'More actions' && el.tagName === 'BUTTON')
  if (!more) throw new Error('no More actions button')
  await React.act(async () => { click(dom.document.body.firstChild as MiniElement, more) })
  await settle()
  const save = allElements(dom.document.body).find((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').replace(/\s+/g, ' ').trim() === 'Save to PC…')
  if (!save) throw new Error('no Save to PC… menu row')
  await React.act(async () => { click(dom.document.body.firstChild as MiniElement, save) })
  await settle()
  const text = pane.text()
  expect({
    called: calls.filter((c) => c.cmd === 'drive_save_copy').map((c) => c.args),
    saved: text.includes('Saved a copy'),
  }).toEqual({ called: [{ id: 'photo-1' }], saved: true })
  await pane.unmount()
})
