/**
 * The popover's behaviour (task 1683 slice 3; spec section 9 "Popover polling", section 6
 * rules 3, 4 and 9, section 7). The controller is plain TypeScript with injected ports, so
 * every claim below is about the real class driven by a scripted backend.
 *
 * The rule under test: NOTHING runs while the popover is hidden. The window is created once
 * and kept alive, so any call made at construction, on a timer or on an event while hidden
 * is the polling the old `tray` window was flagged for.
 *
 * Mutation checks (red first, then reverted) are recorded in the task Notes, 2026-10-01.
 */
import { describe, expect, test } from 'bun:test'
import type { CommandResult } from '../src/desktopApi'
import { parsePopoverSnapshot, type PopoverSnapshot } from '../src/popoverContract'
import { PopoverController, currentView, type Ports } from '../src/macPopover/controller'
import { PENDING_COMMANDS } from '../src/macPopover/commands'
// @ts-expect-error - a plain .mjs shared with the Playwright rung
import { SNAPSHOTS, NOW } from './fixtures/macPopoverStates.mjs'

type Reply = CommandResult<unknown> | Promise<CommandResult<unknown>>
const ok = (value: unknown = null): CommandResult<unknown> => ({ ok: true, value })
const err = (reason: string, unsupported = false): CommandResult<unknown> => ({ ok: false, reason, unsupported })
const snapshot = (key: string): PopoverSnapshot => parsePopoverSnapshot(JSON.parse(JSON.stringify(SNAPSHOTS[key])))!

function harness(opts: { snapshot?: string; replies?: Record<string, Reply | (() => Reply)> } = {}) {
  const log: string[] = []
  const args: Record<string, unknown[]> = {}
  let current = snapshot(opts.snapshot ?? 'synced')
  let snapshotReply: (() => Promise<CommandResult<PopoverSnapshot>>) | null = null
  const ports: Ports = {
    loadSnapshot: async () => {
      log.push('snapshot')
      return snapshotReply ? snapshotReply() : { ok: true, value: current }
    },
    call: async (name, a) => {
      log.push(name)
      ;(args[name] ??= []).push(a)
      const reply = opts.replies?.[name]
      return (typeof reply === 'function' ? reply() : reply) ?? ok()
    },
    openUrl: async (url) => {
      log.push(`open_url ${url}`)
      return (opts.replies?.open_url as CommandResult<void>) ?? { ok: true, value: undefined }
    },
    hideWindow: async () => {
      log.push('hide')
    },
    refreshTheme: async () => {
      log.push('theme')
    },
  }
  const controller = new PopoverController(ports)
  return {
    controller, log, args, ports,
    setSnapshot: (key: string) => { current = snapshot(key) },
    setSnapshotReply: (fn: typeof snapshotReply) => { snapshotReply = fn },
    flush: async () => { for (let i = 0; i < 8; i++) await Promise.resolve() },
  }
}

describe('silence while hidden', () => {
  test('constructing the controller makes no call', () => {
    const h = harness()
    expect(h.log).toEqual([])
    expect(h.controller.getState().visible).toBe(false)
  })

  test('engine-status events while it was never shown make no call (20 events)', async () => {
    const h = harness()
    for (let i = 0; i < 20; i++) h.controller.engineStatus()
    await h.flush()
    expect(h.log).toEqual([])
  })

  test('shown reads the theme once and the snapshot once', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    expect([...h.log].sort()).toEqual(['snapshot', 'theme'])
    expect(h.controller.getState().snapshot?.phase).toBe('synced')
  })

  test('after hidden, engine-status makes no call until the next shown (20 events), then one snapshot', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    h.controller.hidden()
    h.log.length = 0
    for (let i = 0; i < 20; i++) h.controller.engineStatus()
    await h.flush()
    expect(h.log).toEqual([])
    h.controller.shown()
    await h.flush()
    expect([...h.log].sort()).toEqual(['snapshot', 'theme'])
  })

  test('going hidden makes no call itself (blur is not a trigger)', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    const before = h.log.length
    h.controller.hidden()
    await h.flush()
    expect(h.log.length).toBe(before)
    expect(h.controller.getState().visible).toBe(false)
  })

  test('refresh() called directly while hidden still does nothing', async () => {
    const h = harness()
    await h.controller.refresh()
    expect(h.log).toEqual([])
  })

  test('there is no timer: a visible popover stays quiet between events', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await new Promise((r) => setTimeout(r, 120))
    expect(h.log).toEqual([])
  })

  test('Esc hides the window once, makes the popover hidden, and fetches nothing', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await h.controller.escape()
    h.controller.engineStatus()
    await h.flush()
    expect(h.log).toEqual(['hide'])
    expect(h.controller.getState().visible).toBe(false)
  })
})

describe('refreshing while visible', () => {
  test('an engine-status event refreshes the snapshot', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    h.setSnapshot('paused')
    h.controller.engineStatus()
    await h.flush()
    expect(h.controller.getState().snapshot?.phase).toBe('paused')
    expect(h.log.filter((l) => l === 'snapshot').length).toBe(2)
  })

  test('a burst of 6 events during one slow fetch costs exactly one more fetch (coalesced)', async () => {
    const h = harness()
    let release!: () => void
    const gate = new Promise<void>((r) => { release = r })
    h.setSnapshotReply(async () => { await gate; return { ok: true, value: snapshot('synced') } })
    h.controller.shown()
    await h.flush()
    for (let i = 0; i < 6; i++) h.controller.engineStatus()
    release()
    await h.flush()
    await h.flush()
    expect(h.log.filter((l) => l === 'snapshot').length).toBe(2)
  })

  test('a failed fetch keeps the last good snapshot and is drawn only when there is none', async () => {
    const h = harness()
    h.controller.shown()
    await h.flush()
    h.setSnapshotReply(async () => err('boom'))
    h.controller.engineStatus()
    await h.flush()
    const state = h.controller.getState()
    expect(state.loadFailed).toBe(true)
    expect(currentView(state, NOW, 'en').state).toBe('synced') // still the last good one

    const cold = harness()
    cold.setSnapshotReply(async () => err('boom'))
    cold.controller.shown()
    await cold.flush()
    expect(currentView(cold.controller.getState(), NOW, 'en').state).toBe('loadfail')
    expect(currentView(harness().controller.getState(), NOW, 'en').state).toBe('loading')
  })

  test('shown resets the unlock form and any notice from the last time', async () => {
    const h = harness({ snapshot: 'locked' })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'unlock_open' })
    expect(h.controller.getState().unlockOpen).toBe(true)
    h.controller.hidden()
    h.controller.shown()
    expect(h.controller.getState().unlockOpen).toBe(false)
    expect(h.controller.getState().shownCount).toBe(2)
  })
})

describe('actions call the command the spec names', () => {
  const run = async (key: string, request: Parameters<PopoverController['perform']>[0], replies: Record<string, Reply> = {}) => {
    const h = harness({ snapshot: key, replies })
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await h.controller.perform(request)
    await h.flush()
    return h
  }

  test('Resume sync -> tray_resume_sync, then a refresh', async () => {
    expect((await run('paused', { id: 'resume' })).log).toEqual(['tray_resume_sync', 'snapshot'])
  })

  test('Resume sync that fails is one notice on the popover, no refresh, no hide', async () => {
    const h = await run('paused', { id: 'resume' }, { tray_resume_sync: err('engine busy') })
    expect(h.log).toEqual(['tray_resume_sync'])
    expect(h.controller.getState().notice).toEqual({ text: 'Couldn’t resume sync.', detail: 'engine busy' })
  })

  test('Open in Finder -> open_finder_location with no path (not gated on sync_root), then hide', async () => {
    const h = await run('synced', { id: 'open_finder' })
    expect(h.log).toEqual(['open_finder_location', 'hide'])
    expect(h.args.open_finder_location).toEqual([{ path: null }])
  })

  test('View online and Add storage open their pages, then hide', async () => {
    expect((await run('synced', { id: 'view_online' })).log).toEqual(['open_url https://app.beebeeb.io', 'hide'])
    expect((await run('storage', { id: 'add_storage' })).log).toEqual(['open_url https://app.beebeeb.io/billing', 'hide'])
  })

  test('Set up Beebeeb and Sign in open the onboarding window, then hide', async () => {
    expect((await run('signedout', { id: 'setup' })).log).toEqual(['open_onboarding_window', 'hide'])
    expect((await run('ended', { id: 'sign_in' })).log).toEqual(['open_onboarding_window', 'hide'])
  })

  test('Open System Settings -> open_login_items_and_extensions_settings, then hide', async () => {
    expect((await run('finderoff', { id: 'open_system_settings' })).log).toEqual(['open_login_items_and_extensions_settings', 'hide'])
  })

  test('Review -> open_conflict_window for the file asked for, then hide', async () => {
    const h = await run('conflict', { id: 'review', fileId: 'f1', fileName: 'Budget 2027.xlsx' })
    expect(h.log).toEqual(['open_conflict_window', 'hide'])
    expect(h.args.open_conflict_window).toEqual([{ fileId: 'f1', fileName: 'Budget 2027.xlsx', isText: false }])
  })

  test('a page that will not open is a notice and the popover stays', async () => {
    const h = harness({ snapshot: 'synced', replies: { open_url: err('no browser') } })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'view_online' })
    expect(h.controller.getState().notice).toEqual({ text: 'Couldn’t open the page.', detail: 'no browser' })
    expect(h.log.includes('hide')).toBe(false)
  })

  test('the gear asks Rust for the native menu at its corner and does NOT hide the popover', async () => {
    const h = await run('synced', { id: 'gear', x: 346, y: 56 })
    expect(h.log).toEqual([PENDING_COMMANDS.gearMenu])
    expect(h.args[PENDING_COMMANDS.gearMenu]).toEqual([{ x: 346, y: 56 }])
    expect(h.controller.getState().visible).toBe(true)
  })

  test('a gear menu that cannot open is a notice, once', async () => {
    const h = await run('synced', { id: 'gear', x: 1, y: 2 }, { [PENDING_COMMANDS.gearMenu]: err('Command popover_gear_menu not found') })
    expect(h.controller.getState().notice?.text).toBe('Couldn’t open the menu.')
  })
})

describe('Try again (state d)', () => {
  test('asks for a sync tick, then re-reads the status', async () => {
    const h = harness({ snapshot: 'error' })
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await h.controller.perform({ id: 'retry' })
    expect(h.log).toEqual([PENDING_COMMANDS.retrySync, 'snapshot'])
    expect(h.controller.getState().notice).toBeNull()
  })

  test('while no retry command exists it still re-reads the status and says nothing false', async () => {
    for (const reply of [err('Command popover_retry_sync not found'), err('x', true)]) {
      const h = harness({ snapshot: 'error', replies: { [PENDING_COMMANDS.retrySync]: reply } })
      h.controller.shown()
      await h.flush()
      h.log.length = 0
      await h.controller.perform({ id: 'retry' })
      expect(h.log).toEqual([PENDING_COMMANDS.retrySync, 'snapshot'])
      expect(h.controller.getState().notice).toBeNull()
    }
  })

  test('a retry that really fails is a notice, and the status is still re-read', async () => {
    const h = harness({ snapshot: 'error', replies: { [PENDING_COMMANDS.retrySync]: err('tick failed') } })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'retry' })
    expect(h.controller.getState().notice).toEqual({ text: 'Couldn’t try again.', detail: 'tick failed' })
  })
})

describe('Add to Finder (f, f1, f2)', () => {
  test('f1 shows while the install runs, the command runs once, then the refreshed snapshot decides (D1: a saved failure comes back as an Ok state)', async () => {
    let release!: () => void
    const gate = new Promise<void>((r) => { release = r })
    const h = harness({ snapshot: 'finder', replies: { install_finder_location: () => gate.then(() => ok({ installed: false })) } })
    h.controller.shown()
    await h.flush()
    const pending = h.controller.perform({ id: 'finder_add' })
    await h.flush()
    expect(currentView(h.controller.getState(), NOW, 'en').state).toBe('finderadding')
    // a second press while it runs is ignored
    await h.controller.perform({ id: 'finder_add' })
    h.setSnapshot('finderfail')
    release()
    await pending
    await h.flush()
    expect(h.log.filter((l) => l === 'install_finder_location').length).toBe(1)
    expect(currentView(h.controller.getState(), NOW, 'en').state).toBe('finderfail')
    expect(h.controller.getState().notice).toBeNull() // ONE failure surface: f2, not f2 plus a notice
    expect(h.args.install_finder_location).toEqual([{ path: null }])
  })

  test('an install that is rejected (nothing saved) shows one notice on the state it came from', async () => {
    const h = harness({ snapshot: 'finder', replies: { install_finder_location: err('Finder path invalid') } })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'finder_add' })
    expect(h.controller.getState().notice).toEqual({ text: 'Couldn’t add Beebeeb to Finder.', detail: 'Finder path invalid' })
    expect(h.controller.getState().finderPending).toBe(false)
  })

  test('Try again from f2 runs the same command', async () => {
    const h = harness({ snapshot: 'finderfail' })
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await h.controller.perform({ id: 'finder_retry' })
    expect(h.log).toEqual(['install_finder_location', 'snapshot'])
  })
})

describe('unlock (c1, c1b)', () => {
  const locked = (replies: Record<string, Reply>) => harness({ snapshot: 'locked', replies })

  test('Unlock vault only opens the form; no command runs', async () => {
    const h = locked({})
    h.controller.shown()
    await h.flush()
    h.log.length = 0
    await h.controller.perform({ id: 'unlock_open' })
    expect(h.log).toEqual([])
    expect(currentView(h.controller.getState(), NOW, 'en').state).toBe('unlock')
  })

  test('a refused password is the inline line and the form stays; a later success closes it and refreshes', async () => {
    let attempt = 0
    const h = locked({ [PENDING_COMMANDS.unlockWithPassword]: () => (attempt++ === 0 ? err('wrong_password') : ok()) })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'unlock_open' })
    await h.controller.perform({ id: 'unlock_submit', password: 'nope' })
    expect(h.controller.getState()).toMatchObject({ unlockOpen: true, unlockWrong: true, notice: null })
    h.setSnapshot('synced')
    await h.controller.perform({ id: 'unlock_submit', password: 'right' })
    await h.flush()
    expect(h.controller.getState()).toMatchObject({ unlockOpen: false, unlockWrong: false })
    expect(currentView(h.controller.getState(), NOW, 'en').state).toBe('synced')
  })

  test('the password is passed through once and never kept in the state', async () => {
    const h = locked({})
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'unlock_open' })
    await h.controller.perform({ id: 'unlock_submit', password: 'correct horse battery staple' })
    expect(h.args[PENDING_COMMANDS.unlockWithPassword]).toEqual([{ password: 'correct horse battery staple' }])
    expect(JSON.stringify(h.controller.getState()).includes('correct horse')).toBe(false)
  })

  test('a command that is missing or fails some other way is NOT the wrong-password line (4 failures)', async () => {
    for (const reply of [err('Command popover_unlock_vault not found'), err('x', true), err('keychain locked'), err('network down')]) {
      const h = locked({ [PENDING_COMMANDS.unlockWithPassword]: reply })
      h.controller.shown()
      await h.flush()
      await h.controller.perform({ id: 'unlock_open' })
      await h.controller.perform({ id: 'unlock_submit', password: 'x' })
      const state = h.controller.getState()
      expect(state.unlockWrong).toBe(false)
      expect(state.notice?.text).toBe('Couldn’t unlock the vault.')
      expect(state.unlockOpen).toBe(true)
    }
  })

  test('a second press while the first is running is ignored', async () => {
    let release!: () => void
    const gate = new Promise<void>((r) => { release = r })
    const h = locked({ [PENDING_COMMANDS.unlockWithPassword]: () => gate.then(() => ok()) })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'unlock_open' })
    const first = h.controller.perform({ id: 'unlock_submit', password: 'a' })
    await h.controller.perform({ id: 'unlock_submit', password: 'a' })
    release()
    await first
    expect(h.log.filter((l) => l === PENDING_COMMANDS.unlockWithPassword).length).toBe(1)
  })
})

describe('reload (the load-failed state)', () => {
  test('Try again clears an earlier notice before it re-reads', async () => {
    const h = harness({ snapshot: 'paused', replies: { tray_resume_sync: err('engine busy') } })
    h.controller.shown()
    await h.flush()
    await h.controller.perform({ id: 'resume' })
    expect(h.controller.getState().notice).not.toBeNull()
    await h.controller.perform({ id: 'reload' })
    expect(h.controller.getState().notice).toBeNull()
  })

  test('Try again there re-reads the snapshot and nothing else', async () => {
    const h = harness()
    h.setSnapshotReply(async () => err('boom'))
    h.controller.shown()
    await h.flush()
    h.setSnapshotReply(null)
    h.log.length = 0
    await h.controller.perform({ id: 'reload' })
    expect(h.log).toEqual(['snapshot'])
    expect(currentView(h.controller.getState(), NOW, 'en').state).toBe('synced')
  })
})
