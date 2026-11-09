// Photos lists the account's backups. An empty account says so and offers Add a source. A day group, its count
// and its storage line come from the photos that were listed. A thumbnail that fails shows the file glyph and
// the photo still opens. Selecting and Copy link report the share command's answer. Thumbnails are cached per
// id and are not requested for an id the list does not contain.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { DriveSourceProvider, type DriveItem, type DriveSource, type PhotoEntry } from '@beebeeb/shared/drive-ui'
import { fmtSize } from '../src/v2/sections/files/model'
import { dayHeading, fmtN } from '../src/v2/sections/photos/model'
import { useThumb } from '../src/v2/sections/photos/PhotosView'
import { ToastHost } from '../src/v2/chrome/Toast'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const FILE_GLYPH = 'M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7'
const DISPLAY = 'https://share.example/p/k9m2'
const BANNED = /Anna|Lotte|Harbourlight|Wedding 2025/

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

function button(label: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').replace(/\s+/g, ' ').trim() === label)
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled ${label}`)
  return found[0]!
}

async function settle() {
  for (let i = 0; i < 10; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

type Call = { cmd: string; args?: Record<string, unknown> }

function host(answer: (cmd: string, args?: Record<string, unknown>) => unknown) {
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) { return Promise.resolve(answer(cmd, args)) },
  }
}

function item(id: string, name: string, takenAt: string, size: number, mime = 'image/jpeg'): PhotoEntry {
  const file: DriveItem = {
    id, parentId: 'folder-photos', name, isFolder: false, size, modifiedAt: takenAt, mime,
    shared: { kind: 'none' }, local: 'downloaded', hasThumbnail: true,
  }
  return { item: file, takenAt }
}

function line(entries: PhotoEntry[]): string {
  const photos = entries.filter((e) => !(e.item.mime ?? '').startsWith('video/')).length
  const videos = entries.length - photos
  const bytes = entries.reduce((n, e) => n + (e.item.size ?? 0), 0)
  return `${fmtN(photos)} photos · ${fmtN(videos)} videos · ${fmtSize(bytes)}`
}

function totalsText(): string {
  return allElements(dom.document.body).find((el) => el.hasAttribute('data-totals'))?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
}

function sourceText(): string {
  return ['src/v2/sections/photos/index.tsx', 'src/v2/sections/photos/PhotosView.tsx']
    .map((file) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8'))
    .join('\n')
}

function Ghost({ id }: { id: string }) {
  useThumb(id, 'small')
  return <span data-ghost={id} />
}

function Screen({ source, ghost }: { source: DriveSource; ghost?: string }) {
  const Section = PhotosSection
  return (
    <DriveSourceProvider source={source}>
      <ToastHost>
        <Section nav={{ section: 'photos' } as never} />
        {ghost ? <Ghost id={ghost} /> : null}
      </ToastHost>
    </DriveSourceProvider>
  )
}

async function show(source: DriveSource, ghost?: string) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(<Screen source={source} ghost={ghost} />) })
  await settle()
  return {
    container: container as MiniElement,
    text: () => (dom.document.body.textContent ?? '').replace(/\s+/g, ' ').trim(),
    async unmount() {
      await React.act(async () => { root.unmount() })
      container.parentNode?.removeChild(container)
    },
  }
}

function sourceOf(entries: () => PhotoEntry[], thumbs: string[], opened: string[], failId?: string): DriveSource {
  let listener: ((event: { kind: string }) => void) | null = null
  return {
    kind: 'desktop',
    capabilities: { localCopies: true, revealInFileManager: true, office: false, nativePickers: false },
    photos: async () => ({ entries: entries() }),
    thumbnail: async (id: string) => {
      thumbs.push(id)
      if (id === failId) throw new Error('thumbnail unavailable')
      return `mem://${id}`
    },
    subscribe(next) { listener = next as (event: { kind: string }) => void; return () => { if (listener === next) listener = null } },
    open: async (id: string) => { opened.push(id) },
  } as unknown as DriveSource
}

test('an empty account shows Add a source, not a grid of example photos', async () => {
  clearBody()
  const commands: Call[] = []
  host((cmd, args) => { commands.push({ cmd, args }); if (cmd === 'drive_photo_meta') return {}; if (cmd === 'drive_photo_sources') return []; return null })
  const entries: PhotoEntry[] = []
  const thumbs: string[] = []
  const pane = await show(sourceOf(() => entries, thumbs, []))
  const grid = allElements(dom.document.body).some((el) => el.hasAttribute('data-photo-grid') || el.hasAttribute('data-tile'))
  const empty = allElements(dom.document.body).some((el) => el.getAttribute('data-empty') === 'photos')
  const text = pane.text()
  expect({
    empty,
    addSource: text.includes('Add a source'),
    grid,
    totals: totalsText(),
    banned: BANNED.test(text) || BANNED.test(sourceText()),
    thumbs,
  }).toEqual({
    empty: true,
    addSource: true,
    grid: false,
    totals: line(entries),
    banned: false,
    thumbs: [],
  })
  await React.act(async () => { click(pane.container, button('Add a source')) })
  await settle()
  expect(pane.text()).toContain('No device has backed up photos yet')
  expect(pane.text()).not.toMatch(BANNED)
  expect(commands.some((c) => c.cmd === 'drive_photo_sources')).toBe(true)
  await pane.unmount()
})

test('one day of photos groups that day and the count matches the listing', async () => {
  clearBody()
  host((cmd) => (cmd === 'drive_photo_meta' ? {} : cmd === 'drive_photo_sources' ? [] : null))
  const later = item('day-clock', 'Clock tower.jpg', '2026-04-03T18:05:00', 2_400_000)
  const earlier = item('day-pier', 'North pier.jpg', '2026-04-03T09:10:00', 2_400_000)
  const entries = [earlier, later]
  const thumbs: string[] = []
  const pane = await show(sourceOf(() => entries, thumbs, []))
  const days = allElements(dom.document.body).filter((el) => el.hasAttribute('data-day')).map((el) => el.getAttribute('data-day'))
  const tiles = allElements(dom.document.body).filter((el) => el.hasAttribute('data-tile')).map((el) => el.getAttribute('aria-label'))
  const text = pane.text()
  expect({
    days,
    tiles,
    heading: text.includes(dayHeading('2026-04-03', new Date())),
    totals: totalsText(),
    images: allElements(dom.document.body).filter((el) => el.tagName === 'IMG').length,
    banned: BANNED.test(text),
    meter: allElements(dom.document.body).some((el) => (el.getAttribute('role') === 'meter' || el.getAttribute('role') === 'progressbar') && /amber/.test(el.className)),
  }).toEqual({
    days: ['2026-04-03'],
    tiles: ['Clock tower.jpg', 'North pier.jpg'],
    heading: true,
    totals: line(entries),
    images: 2,
    banned: false,
    meter: false,
  })
  expect(thumbs).toEqual(['day-clock', 'day-pier'])
  await pane.unmount()
})

test('a failed thumbnail shows the file glyph and the photo still opens', async () => {
  clearBody()
  host((cmd) => (cmd === 'drive_photo_meta' ? {} : []))
  const photo = item('thumb-fails', 'Gate latch.jpg', '2026-04-03T11:20:00', 3_200_000)
  const opened: string[] = []
  const pane = await show(sourceOf(() => [photo], [], opened, 'thumb-fails'))
  const tile = allElements(dom.document.body).find((el) => el.getAttribute('data-tile') === 'thumb-fails')
  const inside = tile ? allElements(tile) : []
  const before = {
    glyph: inside.some((el) => el.getAttribute('d') === FILE_GLYPH),
    image: inside.some((el) => el.tagName === 'IMG'),
    pulse: inside.some((el) => el.className.includes('animate-pulse')),
  }
  await React.act(async () => { click(pane.container, tile!) })
  await settle()
  const viewer = allElements(dom.document.body).some((el) => el.getAttribute('data-section') === 'photo-viewer')
  const named = allElements(dom.document.body).some((el) => el.getAttribute('aria-label') === 'Gate latch.jpg')
  expect({ ...before, viewer, named }).toEqual({ glyph: true, image: false, pulse: false, viewer: true, named: true })
  await pane.unmount()
})

test('selecting and copying reports the share answer, and thumbnails are not fetched again or for a missing id', async () => {
  clearBody()
  const commands: Call[] = []
  host((cmd, args) => {
    commands.push({ cmd, args })
    if (cmd === 'drive_photo_meta') return {}
    if (cmd === 'drive_photo_sources') return []
    if (cmd === 'share_link_copy') return { display: DISPLAY }
    return null
  })
  const photo = item('share-one', 'Blue notebook.jpg', '2026-04-03T14:00:00', 3_250_000)
  const thumbs: string[] = []
  const ghost = 'not-in-the-backup'
  const first = await show(sourceOf(() => [photo], thumbs, []), ghost)
  await first.unmount()
  const second = await show(sourceOf(() => [photo], thumbs, []), ghost)
  let shareError: string | null = null
  try {
    await React.act(async () => { click(second.container, button('Select')) })
    await settle()
    const tile = allElements(dom.document.body).find((el) => el.getAttribute('data-tile') === 'share-one')
    await React.act(async () => { click(second.container, tile!) })
    await settle()
    await React.act(async () => { click(second.container, button('Share…')) })
    await settle()
    await React.act(async () => { click(second.container, button('Copy link')) })
    await settle()
  } catch (error) {
    shareError = error instanceof Error ? error.message : String(error)
  }
  const share = commands.find((c) => c.cmd === 'share_link_copy')
  expect({
    thumbs,
    shareError,
    itemId: share?.args?.itemId ?? null,
    toast: second.text().includes(DISPLAY),
    banned: BANNED.test(second.text()),
  }).toEqual({
    thumbs: ['share-one'],
    shareError: null,
    itemId: 'share-one',
    toast: true,
    banned: false,
  })
  await second.unmount()
})
