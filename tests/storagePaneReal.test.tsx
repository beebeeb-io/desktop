// Settings › Storage reads the account summary and the free-up answer. A missing read says so.
// No board example (3.1 GB, 18.4 GB, 240 GB) stands in for a read that did not answer.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { DEFAULT_CONFIG } from '../src/desktopApi'
import { actionFailure } from '../src/v2/sections/files/model'
import { registerBoard } from '../src/v2/board'
import { fmtBytes, GB, MB } from '../src/v2/sections/settings/shell/kit'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const USED = 12 * GB
const QUOTA = 50 * GB
const FILES = 7 * GB
const PHOTOS = 4 * GB
const BACKUPS = 1 * GB
const CACHE = 2 * GB
const PINNED = 512 * MB
const LIMIT = 12 * GB
const ACCOUNT_LINE = `${fmtBytes(USED)} used of ${fmtBytes(QUOTA)}`
const DOWNLOAD_LINE = `${fmtBytes(CACHE)} of ${fmtBytes(LIMIT)}`
const EMPTY_ACCOUNT = actionFailure('read your account’s storage', 'failed').text

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

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

type Call = { cmd: string; args?: Record<string, unknown> }

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

function button(label: string): MiniElement {
  const found = allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === label)
  if (found.length !== 1) throw new Error(`${found.length} buttons labelled ${label}`)
  return found[0]!
}

/** Set the input the way a person does: past React's value tracker, then the input event. */
function setRange(container: MiniElement, input: MiniElement, value: string) {
  const desc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')
  desc?.set?.call(input, value)
  const event = {
    type: 'input',
    target: input,
    srcElement: input,
    currentTarget: container,
    bubbles: true,
    cancelable: false,
    defaultPrevented: false,
    isTrusted: true,
    preventDefault() { this.defaultPrevented = true },
    stopPropagation() {},
  }
  for (const fn of container.listenersFor('input', true)) fn(event)
  for (const fn of container.listenersFor('input', false)) fn(event)
  event.type = 'change'
  for (const fn of container.listenersFor('change', true)) fn(event)
  for (const fn of container.listenersFor('change', false)) fn(event)
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

function host(invoke: (cmd: string, args?: Record<string, unknown>) => unknown) {
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) { return Promise.resolve(invoke(cmd, args)) },
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

test('the pane shows the account’s used and quota, the reported breakdown, and the saved download limit', async () => {
  clearBody()
  const calls: Call[] = []
  const opened: string[] = []
  let saved = { ...DEFAULT_CONFIG, local_cache_limit_bytes: LIMIT, download_kbps_limit: 8_000 }
  host((cmd, args) => {
    calls.push({ cmd, args })
    if (cmd === 'get_desktop_config') return saved
    if (cmd === 'set_desktop_config') {
      saved = { ...saved, ...(args?.config as object) }
      return null
    }
    if (cmd === 'desktop_storage_summary') return { used_bytes: USED, quota_bytes: QUOTA, cache_bytes: CACHE, pinned_bytes: PINNED }
    throw new Error(`${cmd} refused`)
  })
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { Storage, DownloadLimitDialog } = await import('../src/v2/sections/settings/drive/storage')
  const summary = { usedBytes: USED, quotaBytes: QUOTA, filesBytes: FILES, photosBytes: PHOTOS, backupsBytes: BACKUPS }
  const source = { storage: async () => summary }
  const settings = { pane: 'storage' as const, go: () => {}, dialog: null, open: (id: string) => { opened.push(id) }, close: () => {} }
  const pane = await show(React.createElement(Ctx.Provider, { value: settings },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(Storage))))
  expect(pane.text()).toContain(ACCOUNT_LINE)
  expect(pane.text()).toContain(fmtBytes(FILES))
  expect(pane.text()).toContain(fmtBytes(PHOTOS))
  expect(pane.text()).toContain(fmtBytes(BACKUPS))
  expect(pane.text()).toContain(DOWNLOAD_LINE)
  expect(pane.text()).toContain(fmtBytes(PINNED))
  expect(pane.text()).toContain('Not reported yet')
  const bar = allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'Account storage')
  expect(bar?.getAttribute('aria-valuenow')).toBe('24')
  expect(allElements(bar!).some((el) => /\bbg-amber\b/.test(el.className))).toBe(false)
  await React.act(async () => { click(pane.container, button('Your plan')) })
  expect(opened).toEqual(['plan'])
  await pane.unmount()

  const partial = { storage: async () => ({ used_bytes: 9 * GB, quota_bytes: 40 * GB, percentage: 0.225 }) }
  const bare = await show(React.createElement(Ctx.Provider, { value: settings },
    React.createElement(DriveSourceProvider, { source: partial as never }, React.createElement(Storage))))
  expect(bare.text()).toContain(`${fmtBytes(9 * GB)} used of ${fmtBytes(40 * GB)}`)
  expect(bare.text()).toContain('Not reported yet')
  expect(bare.text()).not.toContain('NaN')
  await bare.unmount()

  const dialogSettings = { ...settings, close: () => {} }
  const dialog = await show(React.createElement(Ctx.Provider, { value: dialogSettings }, React.createElement(DownloadLimitDialog)))
  expect(dialog.text()).toContain('12 GB')
  expect(dialog.text()).not.toContain('50 GB')
  const range = allElements(dom.document.body).find((el) => el.tagName === 'INPUT')
  expect(range).toBeTruthy()
  await React.act(async () => { setRange(dialog.container, range!, '8') })
  await settle()
  expect(dialog.text()).toContain('8 GB')
  await React.act(async () => { click(dialog.container, button('Save')) })
  await settle()
  expect(calls.some((c) => c.cmd === 'set_desktop_config' && (c.args?.config as { local_cache_limit_bytes?: number })?.local_cache_limit_bytes === 8 * GB)).toBe(true)
  expect(calls.some((c) => c.cmd === 'set_desktop_config' && 'download_kbps_limit' in ((c.args?.config as object) ?? {}))).toBe(false)
  expect(saved.download_kbps_limit).toBe(8_000)
  await dialog.unmount()
  const again = await show(React.createElement(Ctx.Provider, { value: dialogSettings }, React.createElement(DownloadLimitDialog)))
  expect(again.text()).toContain('8 GB')
  expect(again.text()).not.toContain('12 GB')
  await again.unmount()
})

test('free up space reports the command’s file count and bytes', async () => {
  clearBody()
  const calls: Call[] = []
  const closed: string[] = []
  let answer: unknown = { bytes_freed: 4 * MB, files_removed: 3 }
  host((cmd) => {
    calls.push({ cmd })
    if (cmd === 'get_desktop_config') return { ...DEFAULT_CONFIG }
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: CACHE, pinned_bytes: 0 }
    if (cmd === 'free_up_space') return answer
    throw new Error(`${cmd} refused`)
  })
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { FreeUpDialog } = await import('../src/v2/sections/settings/drive/storage')
  const settings = { pane: 'storage' as const, go: () => {}, dialog: 'free-up', open: () => {}, close: () => { closed.push('close') } }
  const dialog = await show(React.createElement(Ctx.Provider, { value: settings }, React.createElement(FreeUpDialog)))
  expect(allElements(dom.document.body).some((el) => el.getAttribute('data-testid') === 'free-up-result')).toBe(false)
  await React.act(async () => { click(dialog.container, button('Free up space')) })
  await settle()
  const result = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'free-up-result')
  expect(result?.textContent).toContain('Files: 3')
  expect(result?.textContent).toContain(`Bytes: ${fmtBytes(4 * MB)}`)
  expect(closed).toEqual([])
  expect(calls.filter((c) => c.cmd === 'free_up_space')).toHaveLength(1)
  const free = button('Free up space')
  expect(/\bbg-amber\b/.test(free.className)).toBe(true)
  await dialog.unmount()

  answer = { bytes_freed: 2048 }
  const onlyBytes = await show(React.createElement(Ctx.Provider, { value: settings }, React.createElement(FreeUpDialog)))
  await React.act(async () => { click(onlyBytes.container, button('Free up space')) })
  await settle()
  const second = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'free-up-result')
  expect(second?.textContent).toContain('Files: Not reported yet')
  expect(second?.textContent).toContain(`Bytes: ${fmtBytes(2048)}`)
  expect(second?.textContent).not.toMatch(/Files: \d/)
  await onlyBytes.unmount()

  host((cmd) => {
    if (cmd === 'get_desktop_config') return { ...DEFAULT_CONFIG }
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
    if (cmd === 'free_up_space') throw new Error('disk is busy')
    throw new Error(`${cmd} refused`)
  })
  const failed = await show(React.createElement(Ctx.Provider, { value: settings }, React.createElement(FreeUpDialog)))
  await React.act(async () => { click(failed.container, button('Free up space')) })
  await settle()
  expect(failed.text()).toContain(actionFailure('free up space', 'failed').text)
  expect(allElements(dom.document.body).some((el) => el.getAttribute('data-testid') === 'free-up-result')).toBe(false)
  await failed.unmount()
})

test('a read that does not answer shows the honest empty state, not a board number', async () => {
  clearBody()
  registerBoard({
    previews: { bytes: 3.1 * GB, limitGb: 20, fits: '38,000' },
    config: { ...DEFAULT_CONFIG, local_cache_limit_bytes: 50 * GB },
  })
  host((cmd) => { throw new Error(`${cmd} refused`) })
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { Storage } = await import('../src/v2/sections/settings/drive/storage')
  const source = { storage: async () => { throw new Error('usage refused') } }
  const settings = { pane: 'storage' as const, go: () => {}, dialog: null, open: () => {}, close: () => {} }
  const pane = await show(React.createElement(Ctx.Provider, { value: settings },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(Storage))))
  expect(pane.text()).toContain(EMPTY_ACCOUNT)
  expect(pane.text()).toContain('Not reported yet')
  expect(pane.text()).not.toContain('3.1')
  expect(pane.text()).not.toContain('18.4')
  expect(pane.text()).not.toContain('240 GB')
  expect(pane.text()).not.toContain('38,000')
  expect(pane.text()).not.toContain('Anna')
  expect(pane.text()).not.toContain('NaN')
  expect(pane.text()).not.toContain('50 GB')
  const bar = allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === 'Account storage')
  expect(bar).toBeUndefined()
  const src = readFileSync(new URL('../src/v2/sections/settings/drive/storage.tsx', import.meta.url), 'utf8')
  expect(src).not.toMatch(/\bboard\b/)
  expect(src).toContain("open('plan')")
  expect(src).toContain("'free_up_space'")
  expect(src).not.toMatch(/<Meter[^>]*\bamber\b/)
  await pane.unmount()
})
