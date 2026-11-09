// Settings › Thumbnails reads the saved preview choice and the storage summaries Storage already uses.
// A figure no command reported says "Not reported yet". The board's 3.1 GB, 2.4 GB, 30.9 MB and 213 KB never stand in.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { DEFAULT_CONFIG } from '../src/desktopApi'
import { registerBoard } from '../src/v2/board'
import { fmtBytes, GB, MB } from '../src/v2/sections/settings/shell/kit'
import type { PaneId, SettingsCtx } from '../src/v2/sections/settings/shell/context'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const PREVIEWS = 5 * GB
const FALLBACK = 100 * MB
const GRID = 50 * MB
const ON_PC = 6 * GB
const LIMIT = 10 * GB
const ORIGINAL = 12 * MB
const SAMPLE = 800 * 1024
const PHOTOS = 17

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')
let disk: Record<string, unknown>
let account: object
let local: object
const writes: Record<string, unknown>[] = []

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  const proto = dom.window.HTMLElement.prototype as { querySelector?: (sel: string) => MiniElement | null }
  if (typeof proto.querySelector !== 'function') {
    proto.querySelector = function (this: MiniElement) {
      return allElements(this).find((el) => el !== this && (el.tagName === 'BUTTON' || el.tagName === 'INPUT')) ?? null
    }
  }
})
afterAll(() => dom.restore())

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

function button(label: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === label)
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled ${label}`)
  return found[0]!
}

function radio(group: string, label: string): MiniElement {
  const box = allElements(dom.document.body).find((el) => el.getAttribute('role') === 'radiogroup' && el.getAttribute('aria-label') === group)
  const found = box ? allElements(box).filter((el) => el.getAttribute('role') === 'radio' && (el.textContent ?? '').trim() === label) : []
  if (found.length !== 1) throw new Error(`${found.length} radios labelled ${label} in ${group}`)
  return found[0]!
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

function host() {
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) {
      if (cmd === 'get_desktop_config') return disk
      if (cmd === 'set_desktop_config') {
        const patch = { ...((args?.config ?? {}) as Record<string, unknown>) }
        writes.push(patch)
        Object.assign(disk, patch)
        return null
      }
      if (cmd === 'desktop_storage_summary') return local
      throw new Error(`${cmd} refused`)
    },
  }
}

async function show(node: import('react').ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
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

async function mount(which: PaneId = 'thumbnails') {
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { Thumbnails, Compare } = await import('../src/v2/sections/settings/drive/thumbnails')
  function Host() {
    const [pane, setPane] = React.useState<PaneId>(which)
    const [dialog, setDialog] = React.useState<string | null>(null)
    const settings: SettingsCtx = {
      pane,
      go: (next) => { setPane(next); setDialog(null) },
      dialog,
      open: (id) => setDialog(id),
      close: () => setDialog(null),
    }
    const source = { storage: async () => account }
    const view = pane === 'compare' ? React.createElement(Compare) : React.createElement(Thumbnails)
    return React.createElement(Ctx.Provider, { value: settings },
      React.createElement(DriveSourceProvider, { source: source as never }, view))
  }
  return show(React.createElement(Host))
}

const BANNED = ['3.1 GB', '2.4 GB', '30.9 MB', '213 KB', '420 KB', '640 KB', '1.0 MB', '2,431']

test('saved preview size and format round-trip, and Cancel writes nothing', async () => {
  clearBody()
  writes.length = 0
  disk = { ...DEFAULT_CONFIG, download_kbps_limit: 8_000, photo_previews: false, preview_size: '2K', preview_format: 'JPEG' }
  account = { usedBytes: 0, quotaBytes: 0, filesBytes: 0, photosBytes: 0, backupsBytes: 0 }
  local = { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
  host()
  const pane = await mount()
  expect(radio('Preview size', '2K').getAttribute('aria-checked')).toBe('true')
  expect(radio('Format', 'JPEG').getAttribute('aria-checked')).toBe('true')
  expect(radio('Preview size', '4K').getAttribute('aria-checked')).toBe('false')
  const sw = allElements(dom.document.body).find((el) => el.getAttribute('role') === 'switch' && el.getAttribute('aria-label') === 'Photo previews')
  expect(sw?.getAttribute('aria-checked')).toBe('false')
  expect(pane.text()).toContain('2K JPEG · Not reported yet')
  expect(pane.text()).toContain('Not reported yet')
  for (const banned of BANNED) expect(pane.text()).not.toContain(banned)

  await React.act(async () => { click(pane.container, radio('Preview size', '720p')) })
  await settle()
  expect(pane.text()).toContain('Nothing changes until Apply. The old previews stay until the new ones have uploaded.')
  expect(pane.text()).toContain('New previews:')
  expect(pane.text()).toContain('Current previews:')
  expect(pane.text()).toContain('Not reported yet')
  const apply = button('Apply')
  expect(/\bbg-amber\b/.test(apply.className)).toBe(true)
  expect(/\bbg-amber\b/.test(button('Cancel').className)).toBe(false)
  await React.act(async () => { click(pane.container, button('Cancel')) })
  await settle()
  expect(writes).toEqual([])
  expect(radio('Preview size', '2K').getAttribute('aria-checked')).toBe('true')
  expect(pane.text()).not.toContain('Apply 720p JPEG?')

  await React.act(async () => { click(pane.container, radio('Preview size', '720p')) })
  await settle()
  await React.act(async () => { click(pane.container, button('Apply')) })
  await settle()
  expect(writes).toEqual([{ preview_size: '720p' }])
  expect(disk.download_kbps_limit).toBe(8_000)
  expect(radio('Preview size', '720p').getAttribute('aria-checked')).toBe('true')

  await React.act(async () => { click(pane.container, radio('Format', 'HEIC')) })
  await settle()
  await React.act(async () => { click(pane.container, button('Apply')) })
  await settle()
  expect(writes[1]).toEqual({ preview_format: 'HEIC' })
  expect(radio('Format', 'HEIC').getAttribute('aria-checked')).toBe('true')

  const toggle = allElements(dom.document.body).find((el) => el.getAttribute('role') === 'switch' && el.getAttribute('aria-label') === 'Photo previews')
  await React.act(async () => { click(pane.container, toggle!) })
  await settle()
  expect(writes[2]).toEqual({ photo_previews: true })
  expect(disk.preview_size).toBe('720p')
  expect(disk.preview_format).toBe('HEIC')
  await pane.unmount()

  const again = await mount()
  expect(radio('Preview size', '720p').getAttribute('aria-checked')).toBe('true')
  expect(radio('Format', 'HEIC').getAttribute('aria-checked')).toBe('true')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'Photo previews')?.getAttribute('aria-checked')).toBe('true')
  expect(again.text()).not.toContain('213 KB')
  await again.unmount()
})

test('preview usage comes from the storage summaries, and the meter is not amber', async () => {
  clearBody()
  writes.length = 0
  disk = { ...DEFAULT_CONFIG, photo_previews: true, preview_size: '2K', preview_format: 'JPEG' }
  account = {
    usedBytes: 0, quotaBytes: 0, filesBytes: 0, photosBytes: 0, backupsBytes: 0,
    previewBytes: PREVIEWS, fallbackPreviewBytes: FALLBACK, thumbnailBytes: GRID,
    originalBytes: ORIGINAL, samplePreviewBytes: SAMPLE, photoCount: PHOTOS,
  }
  local = { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0, local_preview_bytes: ON_PC, preview_limit_bytes: LIMIT }
  host()
  const pane = await mount()
  expect(pane.text()).toContain(fmtBytes(PREVIEWS))
  expect(pane.text()).toContain(fmtBytes(FALLBACK))
  expect(pane.text()).toContain(fmtBytes(GRID))
  expect(pane.text()).toContain(`${fmtBytes(ON_PC)} of ${fmtBytes(LIMIT)}`)
  expect(pane.text()).toContain(`2K JPEG · ${fmtBytes(SAMPLE)}, original ${fmtBytes(ORIGINAL)}`)
  for (const banned of BANNED) expect(pane.text()).not.toContain(banned)
  const bar = allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'Preview storage')
  expect(bar?.getAttribute('aria-valuenow')).toBe('60')
  expect(allElements(bar!).some((el) => /\bbg-amber\b/.test(el.className))).toBe(false)

  await React.act(async () => { click(pane.container, button('Compare')) })
  await settle()
  expect(pane.text()).toContain(`2K JPEG against original ${fmtBytes(ORIGINAL)}`)
  expect(pane.text()).not.toContain('is missing')
  expect(pane.text()).not.toContain('30.9 MB')
  expect(/\bbg-amber\b/.test(button('Done').className)).toBe(true)
  await React.act(async () => { click(pane.container, button('Done')) })
  await settle()

  await React.act(async () => { click(pane.container, radio('Preview size', '720p')) })
  await settle()
  expect(pane.text()).toContain('Apply 720p JPEG to 17 photos?')
  expect(pane.text()).toContain(`Current previews: ${fmtBytes(PREVIEWS)}`)
  expect(pane.text()).toContain('New previews:')
  expect(pane.text()).toContain('Nothing changes until Apply. The old previews stay until the new ones have uploaded.')
  expect(pane.text()).not.toContain('2,431')
  await React.act(async () => { click(pane.container, button('Cancel')) })
  await settle()
  expect(writes).toEqual([])
  expect(radio('Preview size', '2K').getAttribute('aria-checked')).toBe('true')
  await pane.unmount()
})

test('an empty read renders none of the board sizes, and compare says which value is missing', async () => {
  clearBody()
  writes.length = 0
  registerBoard({
    previews: { bytes: 3.1 * GB, limitGb: 20, fits: '38,000' },
    config: { ...DEFAULT_CONFIG, preview_size: '4K', preview_format: 'HEIC' },
  })
  disk = { ...DEFAULT_CONFIG }
  account = { usedBytes: 0, quotaBytes: 0, filesBytes: 0, photosBytes: 0, backupsBytes: 0 }
  local = { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
  host()
  const pane = await mount()
  expect(pane.text()).toContain('Not reported yet')
  for (const banned of BANNED) expect(pane.text()).not.toContain(banned)
  expect(pane.text()).not.toContain('420 MB')
  expect(pane.text()).not.toContain('300 MB')
  expect(pane.text()).not.toContain('20 GB')
  expect(pane.text()).not.toContain('38,000')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'Preview storage')).toBeUndefined()

  await React.act(async () => { click(pane.container, button('Compare')) })
  await settle()
  expect(pane.text()).toContain('Preview size is missing')
  expect(pane.text()).toContain('Preview format is missing')
  expect(pane.text()).toContain('Original size is missing')
  expect(pane.text()).not.toContain('against original')
  for (const banned of BANNED) expect(pane.text()).not.toContain(banned)
  await pane.unmount()

  disk = {}
  const refused = { storage: async () => { throw new Error('usage refused') } }
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string) { throw new Error(`${cmd} refused`) },
  }
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { Thumbnails } = await import('../src/v2/sections/settings/drive/thumbnails')
  const settings: SettingsCtx = { pane: 'thumbnails', go: () => {}, dialog: null, open: () => {}, close: () => {} }
  const failed = await show(React.createElement(Ctx.Provider, { value: settings },
    React.createElement(DriveSourceProvider, { source: refused as never }, React.createElement(Thumbnails))))
  expect(failed.text()).toContain('Not reported yet')
  for (const banned of BANNED) expect(failed.text()).not.toContain(banned)
  await failed.unmount()

  const src = readFileSync(new URL('../src/v2/sections/settings/drive/thumbnails.tsx', import.meta.url), 'utf8')
  expect(src).not.toMatch(/\bboard\b/)
  expect(src).not.toMatch(/from ['"][^'"]*fixtures/)
  for (const banned of ['3.1 GB', '2.4 GB', '30.9 MB', '213 KB', '420 KB', '640 KB', '1.0 MB', '2,431', 'Rebuilding previews']) expect(src).not.toContain(banned)
})
