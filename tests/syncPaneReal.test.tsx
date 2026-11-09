// Sync settings show the engine, one fact per command. statusValue and unsentLine are pure and name
// running, paused, stopped and refused. A count appears only when that command returned one.
// Run by `bun test tests/syncPaneReal.test.tsx`. No production module imports this file.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { DEFAULT_CONFIG, type FinderInstallState } from '../src/desktopApi'
import { actionFailure } from '../src/v2/sections/files/model'
import { allElements, click, installMiniDom, type MiniDomInstall, type MiniElement } from './fixtures/miniDom'

const READ_FAILED = 'Beebeeb couldn\u2019t read how many changes have not uploaded yet.'
const ZERO = 'Every change made on this PC has uploaded.'

function countLine(total: number): string {
  const changes = total === 1 ? 'change' : 'changes'
  const has = total === 1 ? 'has' : 'have'
  return `${total.toLocaleString('en')} ${changes} made on this PC ${has} not uploaded yet and would be lost.`
}

const todaySrc = readFileSync(new URL('../src/v2/sections/settings/today/sync.tsx', import.meta.url), 'utf8')
const driveSrc = readFileSync(new URL('../src/v2/sections/settings/drive/sync.tsx', import.meta.url), 'utf8')

function fnBody(src: string, name: string): string {
  const start = src.indexOf(`export function ${name}`)
  const next = src.indexOf('\nexport function ', start + 1)
  return start < 0 ? '' : src.slice(start, next === -1 ? undefined : next)
}

let dom: MiniDomInstall
let React: typeof import('react')
let client: typeof import('react-dom/client')

beforeAll(async () => {
  dom = installMiniDom('')
  React = await import('react')
  client = await import('react-dom/client')
  // Dialog focuses its first button. The mini document has querySelectorAll only.
  const proto = dom.window.HTMLElement.prototype as { querySelector?: (sel: string) => MiniElement | null }
  if (typeof proto.querySelector !== 'function') {
    proto.querySelector = function (this: MiniElement) {
      return allElements(this).find((el) => el !== this && (el.tagName === 'BUTTON' || el.tagName === 'INPUT')) ?? null
    }
  }
})
afterAll(() => dom.restore())

type Call = { cmd: string; args?: Record<string, unknown> }
type Answer = (cmd: string, args?: Record<string, unknown>) => unknown

function clearBody() {
  for (const child of [...dom.document.body.childNodes]) dom.document.body.removeChild(child)
}

function host(answer: Answer): Call[] {
  const calls: Call[] = []
  dom.window.__TAURI_INTERNALS__ = {
    transformCallback() { return 1 },
    invoke(cmd: string, args?: Record<string, unknown>) {
      calls.push({ cmd, args })
      return Promise.resolve().then(() => answer(cmd, args))
    },
  }
  return calls
}

function text(): string {
  return (dom.document.body.textContent ?? '').replace(/\s+/g, ' ')
}

function buttons(label: string): MiniElement[] {
  return allElements(dom.document.body).filter((el) => el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === label)
}

function offers(): string[] {
  return ['Install', 'Repair', 'Off'].filter((label) => buttons(label).length === 1)
}

function byLabel(label: string): MiniElement | undefined {
  return allElements(dom.document.body).find((el) => el.getAttribute('aria-label') === label)
}

function historyLinks(): MiniElement[] {
  return allElements(dom.document.body).filter((el) => el.tagName === 'A' && (el.getAttribute('href') ?? '').includes('section=sync-history'))
}

function amberTexts(): string[] {
  return allElements(dom.document.body)
    .filter((el) => /\bbg-amber\b/.test(el.className ?? ''))
    .map((el) => (el.textContent ?? '').trim())
}

async function settle() {
  for (let i = 0; i < 8; i++) await React.act(async () => { await new Promise((r) => setTimeout(r, 0)) })
}

async function show(node: import('react').ReactElement) {
  const container = dom.document.createElement('div')
  dom.document.body.appendChild(container)
  const root = client.createRoot(container as never)
  await React.act(async () => { root.render(node) })
  await settle()
  return {
    container: container as MiniElement,
    async unmount() {
      await React.act(async () => root.unmount())
      container.parentNode?.removeChild(container)
    },
  }
}

const source = {
  sync: async () => ({ phase: 'up-to-date' as const, onDevice: 1, availableOffline: 0, onlineOnly: 0, pending: 12345 }),
  subscribe: () => () => {},
}

function settings(over: { open?: (id: string) => void } = {}) {
  return { pane: 'sync' as const, go: () => {}, dialog: null, open: over.open ?? (() => {}), close: () => {} }
}

function shell(over: Partial<FinderInstallState>): FinderInstallState {
  return { installed: false, path: null, status: 'missing', last_error: null, ...over }
}

function config(pause: boolean) {
  return { ...DEFAULT_CONFIG, pause_sync: pause, sync_overlays: false }
}

test('statusValue and unsentLine cover running, paused, stopped and refused', async () => {
  const { statusValue } = await import('../src/v2/sections/settings/today/sync')
  const { unsentLine } = await import('../src/v2/sections/settings/drive/sync')
  const statusBody = fnBody(todaySrc, 'statusValue')
  const unsentBody = fnBody(driveSrc, 'unsentLine')
  expect(statusBody.length).toBeGreaterThan(0)
  expect(unsentBody.length).toBeGreaterThan(0)
  expect(statusBody).not.toMatch(/\b(useState|useEffect|command)\b/)
  expect(unsentBody).not.toMatch(/\b(useState|useEffect|command)\b/)

  const call = (state: string) => {
    try { return statusValue(state as never) } catch (error) { return `threw ${error instanceof Error ? error.message : error}` }
  }
  expect({
    running: call('running'),
    paused: call('paused'),
    stopped: call('stopped'),
    refused: call('refused'),
    unread: call(''),
  }).toEqual({ running: 'Running', paused: 'Paused', stopped: 'Stopped', refused: 'Refused', unread: undefined })
  for (const word of ['Running', 'Paused', 'Stopped', 'Refused']) expect(word).not.toMatch(/\d/)

  const line = (state: string, total: number | null) => {
    try { return unsentLine(state as never, total as never) } catch (error) { return `threw ${error instanceof Error ? error.message : error}` }
  }
  const two = countLine(2)
  const one = countLine(1)
  expect({
    running: line('running', 2),
    paused: line('paused', 2),
    stopped: line('stopped', 1),
    refused: line('refused', 9),
    runningZero: line('running', 0),
    pausedZero: line('paused', 0),
    stoppedZero: line('stopped', 0),
    refusedZero: line('refused', 0),
    runningUnread: line('running', null),
    pausedUnread: line('paused', null),
    stoppedUnread: line('stopped', null),
  }).toEqual({
    running: two,
    paused: `Paused. ${two}`,
    stopped: `Stopped. ${one}`,
    refused: READ_FAILED,
    runningZero: ZERO,
    pausedZero: `Paused. ${ZERO}`,
    stoppedZero: `Stopped. ${ZERO}`,
    refusedZero: READ_FAILED,
    runningUnread: READ_FAILED,
    pausedUnread: READ_FAILED,
    stoppedUnread: READ_FAILED,
  })
  expect(line('refused', 9)).not.toMatch(/\d/)
  expect(line('running', 2)).not.toContain('14 MB')
  expect(line('running', 2)).not.toContain('files')
  expect(line('paused', 1000)).toBe(`Paused. ${countLine(1000)}`)
  expect(line('running', 2)).toBe(line('running', 2))
})

test('status, Explorer, unsent changes and the folder each come from their own command', async () => {
  for (const name of ['autostart_enabled', 'toggle_autostart', 'install_windows_shell_integration', 'windows_shell_integration_state', 'open_finder_location']) {
    expect(todaySrc).toContain(`'${name}'`)
  }
  expect(driveSrc).toContain("'unsent_changes'")
  expect(driveSrc).toContain('function ExplorerOffDialog')
  clearBody()
  const calls = host((cmd) => {
    if (cmd === 'get_desktop_config') return config(false)
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
    if (cmd === 'autostart_enabled') return false
    if (cmd === 'sync_status') return { logged_in: true, engine: 'stopped', sync_root: 'C:\\Wrong\\Root', syncing: 99, cloud_only: 0, conflicts: 0, engine_refusal: null }
    if (cmd === 'windows_shell_integration_state') return shell({ installed: true, path: 'D:\\Beebeeb\\Folder', status: 'installed' })
    if (cmd === 'unsent_changes') return { total: 4, items: [], engine_state: 'running', paused: false }
    throw new Error(`${cmd} refused`)
  })
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { SyncToday } = await import('../src/v2/sections/settings/today/sync')
  const pane = await show(React.createElement(Ctx.Provider, { value: settings(),
  }, React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
  const status = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')
  const place = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'explorer-on')
  const folder = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-folder')
  const unsent = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'unsent-line')
  expect(status?.textContent).toBe('Stopped')
  expect(place?.textContent).toBe('On')
  expect(folder?.textContent).toBe('D:\\Beebeeb\\Folder')
  expect(unsent?.textContent).toBe(countLine(4))
  expect(text()).not.toContain('C:\\Wrong\\Root')
  expect(text()).not.toContain('99')
  expect(text()).not.toContain('12345')
  expect(text()).not.toContain('Anna')
  expect(text()).not.toContain('14 MB')
  expect(byLabel('Open Beebeeb when you log in')?.getAttribute('aria-checked')).toBe('false')
  expect(offers()).toEqual(['Off'])
  await React.act(async () => { click(pane.container, buttons('Show in File Explorer')[0]!) })
  await settle()
  expect(calls.some((c) => c.cmd === 'open_finder_location' && c.args?.path === 'D:\\Beebeeb\\Folder')).toBe(true)
  expect(calls.some((c) => c.cmd === 'open_finder_location' && (c.args?.path == null || c.args.path === 'C:\\Wrong\\Root'))).toBe(false)
  await pane.unmount()

  host((cmd) => {
    if (cmd === 'get_desktop_config') return config(true)
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
    if (cmd === 'autostart_enabled') return false
    if (cmd === 'sync_status') return { logged_in: true, engine: 'running', sync_root: 'C:\\Also\\Wrong', syncing: 8, cloud_only: 0, conflicts: 0, engine_refusal: null }
    if (cmd === 'windows_shell_integration_state') return shell({ installed: false, path: null, status: 'missing' })
    if (cmd === 'unsent_changes') return { total: 0, items: [], engine_state: 'running', paused: true }
    throw new Error(`${cmd} refused`)
  })
  const paused = await show(React.createElement(Ctx.Provider, { value: settings() },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Paused')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'explorer-on')?.textContent).toBe('Off')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-folder')?.textContent).toBe('Not reported')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'unsent-line')?.textContent).toBe(`Paused. ${ZERO}`)
  expect(text()).not.toContain('C:\\Also\\Wrong')
  expect(text()).not.toContain('8')
  expect(historyLinks()).toHaveLength(0)
  await paused.unmount()

  host((cmd) => {
    if (cmd === 'get_desktop_config') return config(false)
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
    if (cmd === 'autostart_enabled') return true
    if (cmd === 'sync_status') throw new Error('sync_status refused')
    if (cmd === 'windows_shell_integration_state') return shell({ installed: true, path: 'D:\\Beebeeb\\Folder', status: 'installed' })
    if (cmd === 'unsent_changes') return { total: 6, items: [], engine_state: 'stopped', paused: false }
    throw new Error(`${cmd} refused`)
  })
  const refused = await show(React.createElement(Ctx.Provider, { value: settings() },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Refused')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'unsent-line')?.textContent).toBe(`Stopped. ${countLine(6)}`)
  expect(text()).not.toContain('99')
  expect(byLabel('Open Beebeeb when you log in')?.getAttribute('aria-checked')).toBe('true')
  await refused.unmount()
})

test('pause writes pause_sync and waits; a refused write shows the error and leaves the switch', async () => {
  clearBody()
  let release: ((value: unknown) => void) | null = null
  let fail = false
  const calls = host((cmd) => {
    if (cmd === 'get_desktop_config') return config(false)
    if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
    if (cmd === 'autostart_enabled') return false
    if (cmd === 'sync_status') return { logged_in: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }
    if (cmd === 'windows_shell_integration_state') return shell({ installed: false, status: 'missing' })
    if (cmd === 'unsent_changes') return { total: 0, items: [], engine_state: 'running', paused: false }
    if (cmd === 'set_desktop_config') {
      return new Promise((resolve, reject) => {
        release = () => { if (fail) reject(new Error('nope')); else resolve(null) }
      })
    }
    throw new Error(`${cmd} refused`)
  })
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { SyncToday } = await import('../src/v2/sections/settings/today/sync')
  const pane = await show(React.createElement(Ctx.Provider, { value: settings() },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
  const toggle = byLabel('Pause sync')
  expect(toggle?.getAttribute('role')).toBe('switch')
  expect(toggle?.getAttribute('aria-checked')).toBe('false')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Running')
  await React.act(async () => { click(pane.container, toggle!) })
  await settle()
  expect(calls.filter((c) => c.cmd === 'set_desktop_config').map((c) => c.args)).toEqual([{ config: { pause_sync: true } }])
  expect(calls.some((c) => c.cmd === 'tray_pause_sync' || c.cmd === 'tray_resume_sync')).toBe(false)
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Running')
  expect(byLabel('Pause sync')?.getAttribute('aria-checked')).toBe('false')
  release?.(null)
  await settle()
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Paused')
  expect(byLabel('Pause sync')?.getAttribute('aria-checked')).toBe('true')
  expect(text()).not.toContain(actionFailure('pause sync', 'failed').text)
  await pane.unmount()

  fail = true
  release = null
  const again = await show(React.createElement(Ctx.Provider, { value: settings() },
    React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
  const second = byLabel('Pause sync')
  expect(second?.getAttribute('aria-checked')).toBe('false')
  await React.act(async () => { click(again.container, second!) })
  await settle()
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Running')
  release?.(null)
  await settle()
  expect(byLabel('Pause sync')?.getAttribute('aria-checked')).toBe('false')
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')?.textContent).toBe('Running')
  expect(text()).toContain(actionFailure('pause sync', 'failed').text)
  expect(text()).not.toContain('nope')
  const writes = calls.filter((c) => c.cmd === 'set_desktop_config').map((c) => c.args)
  expect(writes).toEqual([{ config: { pause_sync: true } }, { config: { pause_sync: true } }])
  await again.unmount()
})

test('unsent changes above zero link to Sync history', async () => {
  clearBody()
  const mount = async (page: unknown, ok = true) => {
    host((cmd) => {
      if (cmd === 'get_desktop_config') return config(false)
      if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
      if (cmd === 'autostart_enabled') return false
      if (cmd === 'sync_status') return { logged_in: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }
      if (cmd === 'windows_shell_integration_state') return shell({ installed: false, status: 'missing' })
      if (cmd === 'unsent_changes') { if (!ok) throw new Error('unsent refused'); return page }
      throw new Error(`${cmd} refused`)
    })
    const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
    const { Ctx } = await import('../src/v2/sections/settings/shell/context')
    const { SyncToday } = await import('../src/v2/sections/settings/today/sync')
    const { ExplorerOffDialog } = await import('../src/v2/sections/settings/drive/sync')
    const pane = await show(React.createElement(Ctx.Provider, { value: settings() },
      React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
    const dialog = await show(React.createElement(Ctx.Provider, { value: settings() }, React.createElement(ExplorerOffDialog)))
    return { pane, dialog }
  }
  const linked = await mount({ total: 3, items: [], engine_state: 'running', paused: false })
  const links = historyLinks()
  expect(links.length).toBeGreaterThanOrEqual(2)
  for (const link of links) expect(link.textContent ?? '').toContain('Sync history')
  expect(text()).toContain(countLine(3))
  await linked.dialog.unmount()
  await linked.pane.unmount()

  const none = await mount({ total: 0, items: [], engine_state: 'running', paused: false })
  expect(historyLinks()).toHaveLength(0)
  expect(text()).toContain(ZERO)
  expect(text()).not.toContain('Sync history')
  await none.dialog.unmount()
  await none.pane.unmount()

  const refused = await mount(null, false)
  expect(historyLinks()).toHaveLength(0)
  expect(text()).toContain(READ_FAILED)
  expect(text()).not.toMatch(/\d/)
  expect(text()).not.toContain('Sync history')
  await refused.dialog.unmount()
  await refused.pane.unmount()
})

test('Explorer offers Install, Repair or Off from the real state, never more than one', async () => {
  const today = await import('../src/v2/sections/settings/today/sync')
  const offer = (today as { explorerOffer?: (state: FinderInstallState | null) => string | null }).explorerOffer
  const seen = (state: FinderInstallState | null) => (typeof offer === 'function' ? offer(state) : `missing:${state?.status ?? 'null'}`)
  expect({
    missing: seen(shell({ status: 'missing' })),
    error: seen(shell({ status: 'error', last_error: 'failed once', path: 'D:\\Beebeeb\\Folder' })),
    installed: seen(shell({ installed: true, status: 'installed', path: 'D:\\Beebeeb\\Folder' })),
    unread: seen(null),
  }).toEqual({ missing: 'install', error: 'repair', installed: 'off', unread: null })

  clearBody()
  const { DriveSourceProvider } = await import('@beebeeb/shared/drive-ui')
  const { Ctx } = await import('../src/v2/sections/settings/shell/context')
  const { SyncToday } = await import('../src/v2/sections/settings/today/sync')
  const draw = async (state: FinderInstallState | null, fail = false) => {
    const calls = host((cmd) => {
      if (cmd === 'get_desktop_config') return config(false)
      if (cmd === 'desktop_storage_summary') return { used_bytes: 0, quota_bytes: 0, cache_bytes: 0, pinned_bytes: 0 }
      if (cmd === 'autostart_enabled') return false
      if (cmd === 'sync_status') return { logged_in: true, engine: 'running', sync_root: 'C:\\Wrong\\Root', syncing: 3, cloud_only: 0, conflicts: 0, engine_refusal: null }
      if (cmd === 'windows_shell_integration_state') { if (fail) throw new Error('shell refused'); return state }
      if (cmd === 'unsent_changes') return { total: 0, items: [], engine_state: 'running', paused: false }
      if (cmd === 'install_windows_shell_integration') return state
      throw new Error(`${cmd} refused`)
    })
    const opened: string[] = []
    const pane = await show(React.createElement(Ctx.Provider, { value: settings({ open: (id) => { opened.push(id) } }) },
      React.createElement(DriveSourceProvider, { source: source as never }, React.createElement(SyncToday))))
    return { calls, opened, pane }
  }
  const statusAmber = () => {
    const status = allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'sync-status')
    expect(status?.className ?? '').not.toMatch(/\bbg-amber\b/)
    expect(allElements(dom.document.body).some((el) => el.getAttribute('role') === 'progressbar')).toBe(false)
  }

  const missing = await draw(shell({ status: 'missing', path: 'D:\\Beebeeb\\Folder' }))
  expect(offers()).toEqual(['Install'])
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'explorer-on')?.textContent).toBe('Off')
  statusAmber()
  expect(amberTexts()).toEqual(['Install'])
  await React.act(async () => { click(missing.pane.container, buttons('Install')[0]!) })
  await settle()
  expect(missing.calls.filter((c) => c.cmd === 'install_windows_shell_integration').map((c) => c.args)).toEqual([{ path: 'D:\\Beebeeb\\Folder' }])
  expect(missing.opened).toEqual([])
  await missing.pane.unmount()

  const broken = await draw(shell({ status: 'error', last_error: 'failed once', path: 'D:\\Beebeeb\\Folder' }))
  expect(offers()).toEqual(['Repair'])
  expect(buttons('Install')).toHaveLength(0)
  expect(buttons('Off')).toHaveLength(0)
  statusAmber()
  expect(amberTexts()).toEqual(['Repair'])
  await React.act(async () => { click(broken.pane.container, buttons('Repair')[0]!) })
  await settle()
  expect(broken.calls.filter((c) => c.cmd === 'install_windows_shell_integration').map((c) => c.args)).toEqual([{ path: 'D:\\Beebeeb\\Folder' }])
  await broken.pane.unmount()

  const on = await draw(shell({ installed: true, status: 'installed', path: 'D:\\Beebeeb\\Folder' }))
  expect(offers()).toEqual(['Off'])
  expect(buttons('Install')).toHaveLength(0)
  expect(buttons('Repair')).toHaveLength(0)
  expect(allElements(dom.document.body).find((el) => el.getAttribute('data-testid') === 'explorer-on')?.textContent).toBe('On')
  statusAmber()
  expect(amberTexts()).toEqual([])
  await React.act(async () => { click(on.pane.container, buttons('Off')[0]!) })
  await settle()
  expect(on.opened).toEqual(['explorer-off'])
  expect(on.calls.some((c) => c.cmd === 'install_windows_shell_integration')).toBe(false)
  await on.pane.unmount()

  const unread = await draw(null, true)
  expect(offers()).toEqual([])
  expect(allElements(dom.document.body).some((el) => el.getAttribute('data-testid') === 'explorer-on')).toBe(false)
  expect(text()).toContain('Beebeeb couldn\u2019t read whether it is in File Explorer.')
  statusAmber()
  expect(amberTexts()).toEqual([])
  await unread.pane.unmount()
})
