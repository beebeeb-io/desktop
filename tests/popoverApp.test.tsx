/**
 * The menu-bar popover (task 1683 slice 3; two-surface spec §6.2-§6.4, §9.3, §11): the real `PopoverApp` and
 * `PopoverSurface` of `src/PopoverApp.tsx`, executed with the component harness against a scripted backend and a
 * scripted event bus.
 *
 * What this proves: it reads `popover_snapshot` and renders the model's view; each action runs its one command; an
 * action whose result the state shows stays busy until the refreshed snapshot is applied; a failed action replaces
 * the body (never a toast) and Try again runs it again; the failure goes with the next show or another phase; no
 * session hides it; the gear asks for the native menu; Esc, ⌘, and ⌘Q. What it does NOT prove: layout
 * (tests/render-popover.mjs), real focus, the native menu, or anything on a Mac.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import { SnapshotReader } from '../src/mainWindowModel'
import { POPOVER_HIDDEN_EVENT, POPOVER_SHOWN_EVENT } from '../src/popoverContract'
import * as app from '../src/PopoverApp'
import { MACOS_DOWNLOAD_URL, popoverView } from '../src/popoverModel'
import { expand, mount, textOf, type Mounted } from './fixtures/componentHarness'

const React = { createElement, Fragment }
// The module's exports are the component's scope; its default export is the component itself.
const { default: _component, ...appScope } = app
const mounted: Mounted[] = []
afterEach(() => {
  // Unmount first: `close` only restores `window`, and a reader left running reads into the next mount's backend.
  while (mounted.length) {
    const m = mounted.pop()!
    m.unmount()
    m.close()
  }
})

const fixture = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/popover/${name}.json`, import.meta.url), 'utf8'))

function eventBus() {
  const listeners = new Map<string, Array<(event: any) => void>>()
  return {
    listen: (name: string, callback: (event: any) => void) => {
      const list = listeners.get(name) ?? []
      list.push(callback)
      listeners.set(name, list)
      return Promise.resolve(() => {})
    },
    emit: (name: string, payload: unknown = null) => { for (const callback of listeners.get(name) ?? []) callback({ payload }) },
  }
}

function openPopover(opts: { snap?: () => unknown; backend?: Record<string, (args: any) => unknown>; bindings?: Record<string, unknown> } = {}) {
  const bus = eventBus()
  const m = mount('PopoverApp.tsx', 'PopoverApp', {
    backend: { popover_snapshot: () => (opts.snap ?? (() => fixture('synced')))(), popover_hide: () => null, ...opts.backend },
    bindings: {
      React,
      ...appScope,
      SnapshotReader,
      POPOVER_SHOWN_EVENT,
      POPOVER_HIDDEN_EVENT,
      popoverView,
      listen: bus.listen,
      command: desktopApi.command,
      ...opts.bindings,
    },
    expand: true,
  })
  mounted.push(m)
  return { m, bus }
}

const text = (m: Mounted) => textOf(expand(m.tree()))
const calls = (m: Mounted, name: string) => m.calls.filter((c) => c.name === name)
const byId = (m: Mounted, id: string) => m.elements().find((e) => e.props.id === id)

describe('reading and rendering', () => {
  test('a dialog labelled Beebeeb that reads the snapshot once at start and shows the model’s view', async () => {
    const { m } = openPopover()
    await m.flush()
    expect(calls(m, 'popover_snapshot').length).toBe(1)
    const root = m.elements().find((e) => e.props.role === 'dialog')
    expect(root?.props['aria-label']).toBe('Beebeeb')
    expect(text(m)).toContain('Recent activity')
    expect(text(m)).toContain('Q3 ledger reconciliation.xlsx')
    expect(text(m)).toContain('Up to date')
    expect(text(m)).toContain('Encrypted')
    expect(text(m)).toContain('Open in Finder')
  })

  test('before the first answer: the busy indicator, no words, and the footer disabled', async () => {
    const { m } = openPopover({ snap: () => new Promise(() => {}) })
    await m.flush()
    expect(m.elements().some((e) => e.props.role === 'status' && e.props['aria-label'] === 'Loading')).toBe(true)
    for (const id of ['open_location', 'view_online', 'add_storage'] as const) expect(byId(m, app.footerId(id))?.props.disabled).toBe(true)
  })

  test('a first read that fails says so once, and Try again reads again', async () => {
    let fail = true
    const { m } = openPopover({ snap: () => { if (fail) throw new Error('nope'); return fixture('synced') } })
    await m.flush()
    expect(text(m)).toContain(app.READ_FAILED)
    fail = false
    await m.click('Try again')
    await m.flush()
    expect(calls(m, 'popover_snapshot').length).toBe(2)
    expect(text(m)).toContain('Recent activity')
  })

  test('popover-shown reads at once', async () => {
    const { m, bus } = openPopover()
    await m.flush()
    bus.emit(POPOVER_SHOWN_EVENT)
    await m.flush()
    expect(calls(m, 'popover_snapshot').length).toBe(2)
  })

  test('hidden, it reads nothing until it is shown again, then reads at once and on its timer (review P2-2 of #160)', async () => {
    // The window is hidden, never destroyed: a timer left running read config, the state DB and (every 5 min while
    // unlocked) the API every 5 s for the app's lifetime after the first click.
    // It counts this popover's own reads: a reader another test file left running reads into this mount's backend too.
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
    let reads = 0
    const readPopoverSnapshot = () => { reads += 1; return app.readPopoverSnapshot() }
    const { m, bus } = openPopover({ bindings: { POPOVER_REFRESH_MS: 5, readPopoverSnapshot } })
    await m.flush()
    await sleep(100)
    expect(reads).toBeGreaterThan(1)
    bus.emit(POPOVER_HIDDEN_EVENT)
    await m.flush()
    const atHide = reads
    await sleep(100)
    expect(reads).toBe(atHide)
    bus.emit(POPOVER_SHOWN_EVENT)
    await m.flush()
    expect(reads).toBe(atHide + 1)
    await sleep(100)
    expect(reads).toBeGreaterThan(atHide + 1)
  })

  test('hidden longer than one refresh, it drops what it showed: shown again, it is busy until the new answer (self-review of #202)', async () => {
    // The window paints before `popover-shown` reaches it, so an answer kept while hidden would show an old state
    // (an hour-old "3 files left") as current. A quick reopen keeps it: it is no older than the timer would have let it be.
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
    const loading = (m: Mounted) => m.elements().some((e) => e.props.role === 'status' && e.props['aria-label'] === 'Loading')
    let release: (() => void) | null = null
    let hold = false
    const snap = () => hold ? new Promise((resolve) => { release = () => resolve(fixture('synced')) }) : fixture('synced')
    const { m, bus } = openPopover({ snap, bindings: { POPOVER_REFRESH_MS: 100 } })
    await m.flush()
    bus.emit(POPOVER_HIDDEN_EVENT)
    await m.flush()
    expect(text(m)).toContain('Recent activity')
    await sleep(250)
    await m.flush()
    expect(loading(m)).toBe(true)
    expect(text(m)).not.toContain('Recent activity')
    hold = true
    bus.emit(POPOVER_SHOWN_EVENT)
    await m.flush()
    expect(loading(m)).toBe(true)
    release!()
    await m.flush()
    expect(text(m)).toContain('Recent activity')
  })

  test('a quick hide and show keeps what it showed and never drops it later (self-review of #202)', async () => {
    const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
    let hold = false
    const snap = () => hold ? new Promise(() => {}) : fixture('synced')
    const { m, bus } = openPopover({ snap, bindings: { POPOVER_REFRESH_MS: 200 } })
    await m.flush()
    bus.emit(POPOVER_HIDDEN_EVENT)
    await sleep(20)
    await m.flush()
    expect(text(m)).toContain('Recent activity')
    bus.emit(POPOVER_SHOWN_EVENT)
    await m.flush()
    // Later reads never answer, so what is on screen after this is what the show's read left there.
    hold = true
    await sleep(300)
    await m.flush()
    expect(text(m)).toContain('Recent activity')
  })

  test('no session: the popover hides itself and shows nothing', async () => {
    const { m } = openPopover({ snap: () => fixture('session-ended') })
    await m.flush()
    // The harness runs the effects a render queued on the next flush; React runs them right after that render.
    await m.flush()
    expect(calls(m, 'popover_hide').length).toBe(1)
    expect(text(m)).toBe('')
  })
})

describe('actions (§6.3, §6.4)', () => {
  const opener = (url: string) => ({ name: 'plugin:opener|open_url', args: { url } })
  const cases: Array<[string, string, { name: string; args?: unknown }]> = [
    ['paused', 'Resume sync', { name: 'tray_resume_sync' }],
    ['offline', 'Try again', { name: 'popover_retry_sync' }],
    ['error', 'Try again', { name: 'popover_retry_sync' }],
    ['locked', 'Unlock vault', { name: 'popover_unlock' }],
    ['storage-full', 'Add storage', opener(desktopApi.BILLING_URL)],
    ['finder-failed', 'Try again', { name: 'finder_setup_retry' }],
    ['finder-user-disabled', 'Open System Settings', { name: 'open_login_items_and_extensions_settings' }],
    ['finder-not-in-applications', 'Show in Finder', { name: 'finder_setup_show_app' }],
    ['needs-decision', 'Review…', { name: 'popover_open_window', args: { to: 'review' } }],
    ['update-available', 'Download update', opener(MACOS_DOWNLOAD_URL)],
    ['sign-out-sending', 'Open Beebeeb', { name: 'popover_open_window', args: { to: 'front' } }],
    ['synced', 'Open in Finder', { name: 'open_finder_location', args: { path: null } }],
    ['synced', 'View online', opener(app.WEB_APP_URL)],
  ]
  for (const [name, label, expected] of cases) {
    test(`${name}: ${label} runs ${expected.name}`, async () => {
      const { m } = openPopover({
        snap: () => fixture(name),
        backend: { [expected.name]: () => null },
      })
      await m.flush()
      await m.click(label)
      const call = calls(m, expected.name)
      expect(call.length).toBe(1)
      if (expected.args !== undefined) expect(call[0].args).toEqual(expected.args)
    })
  }

  test.each(['beta', 'alpha', undefined])('a %s update opens the advertised version’s release', async (channel) => {
    const snap = fixture('update-available')
    snap.update.channel = channel
    const { m } = openPopover({ snap: () => snap, backend: { 'plugin:opener|open_url': () => null } })
    await m.flush()
    await m.click('View release')
    const opened = calls(m, 'plugin:opener|open_url')
    expect(opened.length).toBe(1)
    expect(opened[0].args).toEqual({ url: 'https://github.com/beebeeb-io/desktop/releases/tag/desktop-v0.8.14' })
  })

  test('links go to the browser through the opener only, never window.open (it opens no window but the Beebeeb one)', () => {
    const source = readFileSync(new URL('../src/PopoverApp.tsx', import.meta.url), 'utf8')
    expect(source).not.toMatch(/window\.open\(|openUrl\(|WebviewWindow|new Window/)
  })

  test('an action whose result the state shows stays busy until the refreshed snapshot is applied', async () => {
    let reads = 0
    let answer: (value: unknown) => void = () => {}
    const { m } = openPopover({
      snap: () => { reads += 1; return reads === 1 ? fixture('paused') : new Promise((r) => { answer = r }) },
      backend: { tray_resume_sync: () => null },
    })
    await m.flush()
    void m.clickNoWait('Resume sync')
    await m.flush()
    expect(calls(m, 'tray_resume_sync').length).toBe(1)
    expect(byId(m, app.actionId('resume'))?.props.disabled).toBe(true)
    answer(fixture('synced'))
    await m.flush()
    expect(text(m)).toContain('Recent activity')
    expect(byId(m, app.footerId('open_location'))?.props.disabled).toBe(false)
  })
})

describe('one error surface: a failed action replaces the body, never a toast', () => {
  test('the failure’s sentence and Try again, which runs the action again; no toast', async () => {
    let fail = true
    const { m } = openPopover({
      snap: () => fixture('paused'),
      backend: { tray_resume_sync: () => { if (fail) throw new Error('engine said no'); return null } },
    })
    await m.flush()
    await m.click('Resume sync')
    await m.flush()
    expect(text(m)).toContain('Beebeeb couldn’t resume sync.')
    expect(text(m)).not.toContain('engine said no')
    expect(text(m)).not.toContain('Sync is paused')
    expect(m.toasts).toEqual([])
    fail = false
    await m.click('Try again')
    await m.flush()
    expect(calls(m, 'tray_resume_sync').length).toBe(2)
    expect(text(m)).not.toContain('Beebeeb couldn’t resume sync.')
  })

  test('the failure goes when the popover is shown again', async () => {
    const { m, bus } = openPopover({ snap: () => fixture('paused'), backend: { tray_resume_sync: () => { throw new Error('x') } } })
    await m.flush()
    await m.click('Resume sync')
    await m.flush()
    expect(text(m)).toContain('Beebeeb couldn’t resume sync.')
    bus.emit(POPOVER_SHOWN_EVENT)
    await m.flush()
    expect(text(m)).toContain('Sync is paused')
  })

  test('the failure stays while the phase does, and goes when the state moves to another phase', async () => {
    let name = 'paused'
    const { m } = openPopover({
      snap: () => fixture(name),
      backend: { tray_resume_sync: () => { throw new Error('x') } },
      bindings: { POPOVER_REFRESH_MS: 5 },
    })
    await m.flush()
    await m.click('Resume sync')
    await m.flush()
    expect(text(m)).toContain('Beebeeb couldn’t resume sync.')
    const before = calls(m, 'popover_snapshot').length
    await Bun.sleep(40)
    await m.flush()
    expect(calls(m, 'popover_snapshot').length).toBeGreaterThan(before)
    expect(text(m)).toContain('Beebeeb couldn’t resume sync.')
    name = 'synced'
    await Bun.sleep(40)
    await m.flush()
    expect(text(m)).not.toContain('Beebeeb couldn’t resume sync.')
    expect(text(m)).toContain('Recent activity')
  })

  test('a failed Open in Finder says the shipped sentence (task 17b), and every action has one', () => {
    expect(app.ACTION_FAILED.open_location).toBe('Beebeeb couldn’t open its Finder location.')
    for (const [what, sentence] of Object.entries(app.ACTION_FAILED)) expect([what, sentence.length > 10]).toEqual([what, true])
  })

  test('no two elements share an id, the footer included (review P3-11 of #160)', async () => {
    // Storage full shows Add storage in the body and in the footer: two buttons, one id.
    for (const name of ['storage-full', 'synced', 'syncing', 'paused', 'needs-decision', 'update-available', 'offline', 'error']) {
      const { m } = openPopover({ snap: () => fixture(name) })
      await m.flush()
      const ids = m.elements().map((e) => e.props.id).filter((id): id is string => typeof id === 'string')
      expect([name, ids.length > 0, ids.filter((id, i) => ids.indexOf(id) !== i)]).toEqual([name, true, []])
    }
  })

  test('the popover never mounts a toast host or calls a toast', () => {
    const source = readFileSync(new URL('../src/PopoverApp.tsx', import.meta.url), 'utf8')
    expect(source).not.toMatch(/useToast|ToastProvider|showToast/)
  })
})

describe('the gear, keys and focus (§6.4, §9.3, §11)', () => {
  test('the gear asks Rust for the native menu at the gear’s bottom-left corner', async () => {
    const { m } = openPopover({ backend: { popover_gear_menu: () => null } })
    await m.flush()
    const gear = byId(m, app.GEAR_ID)
    expect(gear?.props['aria-label']).toBe('Beebeeb menu')
    gear!.props.onClick({ currentTarget: { getBoundingClientRect: () => ({ left: 330, bottom: 46 }) } })
    await m.flush()
    expect(calls(m, 'popover_gear_menu')).toEqual([{ name: 'popover_gear_menu', args: { x: 330, y: 46 } }])
  })

  test('Esc hides, ⌘, opens Settings, ⌘Q quits; nothing else is taken', () => {
    expect(app.popoverKey({ key: 'Escape' })).toBe('hide')
    expect(app.popoverKey({ key: ',', metaKey: true })).toBe('settings')
    expect(app.popoverKey({ key: 'q', metaKey: true })).toBe('quit')
    expect(app.popoverKey({ key: 'Q', metaKey: true })).toBe('quit')
    expect(app.popoverKey({ key: 'q' })).toBeNull()
    expect(app.popoverKey({ key: 'q', metaKey: true, shiftKey: true })).toBeNull()
    expect(app.popoverKey({ key: ',', metaKey: true, altKey: true })).toBeNull()
    expect(app.popoverKey({ key: 'w', metaKey: true })).toBeNull()
  })

  test('focus starts on the first primary action, else the gear', () => {
    const at = (name: string) => {
      const screen = popoverView(fixture(name), fixture(name).generated_at, { locale: 'nl', tzOffsetMinutes: 0 })
      return app.focusIdFor(screen.kind === 'state' ? screen : null)
    }
    expect(at('synced')).toBe(app.GEAR_ID)
    expect(at('paused')).toBe(app.actionId('resume'))
    expect(at('update-available')).toBe(app.actionId('download_update'))
    expect(at('needs-decision')).toBe(app.actionId('open_review'))
    expect(app.focusIdFor(null)).toBe(app.GEAR_ID)
  })

  test('a failure moves focus to its Try again, and nothing else shows on top or under it', () => {
    const busy = { ...fixture('update-available'), conflicts: fixture('needs-decision').conflicts }
    const screen = popoverView(busy, busy.generated_at, { locale: 'nl', tzOffsetMinutes: 0 })
    if (screen.kind !== 'state') throw new Error('no state')
    expect([screen.update !== null, screen.attention !== null, screen.status !== null]).toEqual([true, true, true])
    const failed = app.withFailure(screen, { what: 'resume', phase: 'synced' })
    expect(app.focusIdFor(failed)).toBe(app.actionId('resume'))
    expect(failed.body).toMatchObject({ kind: 'message', tone: 'error', title: 'Beebeeb couldn’t resume sync.', action: { label: 'Try again', command: 'resume' } })
    expect([failed.update, failed.attention, failed.status]).toEqual([null, null, null])
  })
})
