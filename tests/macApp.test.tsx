/**
 * The macOS Beebeeb window (task 1683 slice 1; spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7, §8, §11):
 * the real `MacApp`, `HomePane` and `SignedOutCard` declarations of `src/MacApp.tsx`, executed with the component
 * harness against a scripted backend and a scripted event bus.
 *
 * What this proves: the sidebar and its switch to the Settings list (System Settings style, ruling 1), the back
 * control and focus (§11), the keys of both lists, the last section, the one request channel (`main:navigate`), the
 * §8.2 rule that nothing navigates under a sheet and what the window tells Rust for the menu, the root behaviour that
 * moved here from the retired Settings window's tab-strip root (the update-check drain, F9's flow request handled
 * once), Home's first cut, and the signed-out card. The Settings section bodies are stubs here; their own tests are
 * tests/macSettingsTabs.test.tsx, unchanged.
 * What it does NOT prove: layout (tests/render-mac-settings.mjs), React scheduling, real focus, or anything on a Mac.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createElement, Fragment } from 'react'
import * as accountSession from '../src/accountSession'
import * as desktopApi from '../src/desktopApi'
import * as finderSetup from '../src/finderSetup'
import * as finderSetupCopy from '../src/finderSetupCopy'
import * as model from '../src/macSettingsModel'
import * as parts from '../src/macSettingsParts'
import * as windowModel from '../src/mainWindowModel'
import * as reviewModel from '../src/reviewModel'
import * as signOutCopy from '../src/signOutCopy'
import { connectSignOutFlowEvents, connectSignOutFlowMenu, SignOutFlow, signOutFlowApi } from '../src/signOutFlow'
import { connectNativeUpdateMenu as realConnectNativeUpdateMenu } from '../src/windows/manualUpdateCheck'
import { expand, loadComponent, mount, textOf, type Mounted, type TreeNode } from './fixtures/componentHarness'
import { KeptFolderNotes, useKeptFoldersModule } from './fixtures/keptFolderHarness'

const React = { createElement, Fragment }
const mounted: Mounted[] = []
afterEach(() => {
  while (mounted.length) mounted.pop()!.close()
  accountSession.clearSignOutWarning()
})

const showSignOutOutcome = loadComponent('MacSettings.tsx', 'showSignOutOutcome', { stripCode: signOutCopy.stripCode, holdSignOutWarning: accountSession.holdSignOutWarning })
const locationSearch = loadComponent('MacApp.tsx', 'locationSearch', {})
const focusById = (id: string) => { focused.push(id) }
let focused: string[] = []
const accountLine = loadComponent('MacApp.tsx', 'accountLine', { planShortLine: model.planShortLine })
const SignedOutCard = loadComponent('MacApp.tsx', 'SignedOutCard', { React, Btn: parts.Btn, Note: parts.Note })

/** A scripted Tauri event bus: `emit(name)` runs every listener registered for it. */
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
    names: () => [...listeners.keys()].sort(),
  }
}

const snapshot = (over: { account?: Record<string, unknown>; [k: string]: unknown } = {}) => ({
  phase: 'synced', generated_at: 1000, paused: false,
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: Math.floor(Date.now() / 1000) - 30 },
  reason: null, finder: { setup: 'ready', reason: null, reason_line: null },
  storage: { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false },
  storage_full: false, pending_changes: 0, conflicts: { count: 0, files: [] }, activity: [],
  ...over,
  account: { email: 'sam@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false, ...(over.account ?? {}) },
})
const subscription = { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-11-14T00:00:00Z', quota_bytes: 1, used_bytes: 0 }

const tabStub = (id: string) => (props: any) => createElement('div', { 'data-tab': id, 'data-props': props })
const settingsStub = () => ({ state: { status: 'loading' }, save: async () => {}, reload: async () => {} })

interface WindowOpts {
  /** What the Review feed holds (slice 4). Default: loaded and empty. */
  feed?: { status: 'loading' | 'ready' | 'failed'; entries: unknown[]; reason: string | null }
  snap?: () => unknown
  backend?: Record<string, (args: any) => unknown>
  bindings?: Record<string, unknown>
  search?: string
}

/** The window with every section body stubbed; `bus` stands in for Rust's events. */
function openWindow(opts: WindowOpts = {}) {
  const bus = eventBus()
  let pendingNav: unknown = null
  const backend: Record<string, (args: any) => unknown> = {
    popover_snapshot: () => (opts.snap ?? (() => snapshot()))(),
    account_subscription: () => subscription,
    consume_main_navigation: () => { const nav = pendingNav; pendingNav = null; return nav },
    consume_menu_update_check: () => false,
    consume_sign_out_flow_request: () => false,
    main_window_state: () => null,
    ...opts.backend,
  }
  focused = []
  // Slice 4: the Review feed is a module-level store; the window reads it, starts it while there is a session and an
  // unlocked vault, and asks it to read again after a resolved conflict. These record that.
  const feedLog = { connected: 0, disconnected: 0, refreshed: 0, listens: [] as string[] }
  const m = mount('MacApp.tsx', 'MacApp', {
    backend: backend as any,
    expand: true,
    bindings: {
      ...accountSession,
      ...desktopApi,
      ...windowModel,
      SETTINGS_TABS: model.SETTINGS_TABS,
      accountInitial: model.accountInitial,
      planShortLine: model.planShortLine,
      Btn: parts.Btn, Note: parts.Note, SettingsGroup: parts.SettingsGroup, SettingsIcon: parts.SettingsIcon,
      GeneralTab: tabStub('general'), AccountTab: tabStub('account'), SyncTab: tabStub('sync'), AboutTab: tabStub('about'),
      HomePane: tabStub('home'), SignedOutCard,
      ReviewPane: tabStub('review'), TrashPane: tabStub('trash'), LockedPane: tabStub('locked'),
      reviewBadge: reviewModel.reviewBadge,
      reviewFeed: { stub: true },
      subscribeReviewFeed: () => () => {},
      reviewFeedState: () => opts.feed ?? { status: 'ready', entries: [], reason: null },
      connectReviewFeed: () => { feedLog.connected += 1; return () => { feedLog.disconnected += 1 } },
      refreshReviewFeed: () => { feedLog.refreshed += 1 },
      useSettingsConfig: settingsStub,
      listen: bus.listen,
      connectNativeUpdateMenu: () => () => {},
      desktopUpdateCheck: { check: async () => {} },
      SignOutFlow, signOutFlowApi, showSignOutOutcome,
      connectSignOutFlowEvents: () => () => {},
      connectSignOutFlowMenu: () => () => {},
      subscribeOpenModals: () => () => {},
      openModalCount: () => 0,
      locationSearch: () => opts.search ?? '',
      focusById, accountLine,
      SNAPSHOT_REFRESH_MS: 5000,
      ...opts.bindings,
    },
  })
  mounted.push(m)
  /** Rust stores a request and pokes the window, as `show_main_window` does. */
  const request = async (nav: unknown) => { pendingNav = nav; bus.emit(windowModel.MAIN_NAVIGATE_EVENT); await settle(m) }
  return { m, bus, request, feedLog }
}

const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const find = (m: Mounted, match: (el: TreeNode) => boolean) => m.elements().filter(match)
const shown = (m: Mounted) => find(m, (el) => el.props['data-tab'] !== undefined).map((el) => el.props['data-tab'])
const navOf = (m: Mounted) => find(m, (el) => el.type === 'nav')
const buttonsIn = (node: TreeNode) => {
  const out: TreeNode[] = []
  const walk = (n: any) => {
    if (Array.isArray(n)) return n.forEach(walk)
    if (!n?.props) return
    if (n.type === 'button') out.push(n)
    walk(n.props.children)
  }
  walk(node.props.children)
  return out
}
const label = (el: TreeNode) => el.props['aria-label'] ?? textOf(expand(el.props.children)).trim()
const byLabel = (m: Mounted, text: string) => {
  const hit = find(m, (el) => el.type === 'button' && label(el) === text)
  if (hit.length !== 1) throw new Error(`expected one button "${text}", found ${hit.length}: ${find(m, (el) => el.type === 'button').map(label).join(' | ')}`)
  return hit[0]
}
const click = async (m: Mounted, text: string) => { await byLabel(m, text).props.onClick(); m.render(); await settle(m) }
const title = (m: Mounted) => textOf(find(m, (el) => el.props.id === 'mw-title')[0]?.props.children).trim()
const current = (m: Mounted) => find(m, (el) => el.props['aria-current'] === 'page').map(label)
const menuStates = (m: Mounted) => m.calls.filter((c) => c.name === 'main_window_state').map((c) => c.args)

describe('the window, signed in: Home and the sidebar (§7.6)', () => {
  test('Home in the sidebar, selected; the footer has the account and the gear; the toolbar names the section and is the drag region', async () => {
    const { m } = openWindow()
    await settle(m)
    expect(navOf(m).map((n) => n.props['aria-label'])).toEqual(['Beebeeb'])
    expect(buttonsIn(navOf(m)[0]).map(label)).toEqual(['Home', 'Review', 'Trash', 'Settings'])
    expect(current(m)).toEqual(['Home'])
    expect(shown(m)).toEqual(['home'])
    expect(title(m)).toBe('Home')
    const text = textOf(expand(m.tree()))
    expect(text).toContain('sam@example.eu')
    expect(text).toContain('Basic plan')
    expect(byLabel(m, 'Settings').props.id).toBe('mw-gear')
    // Review item 7: the toolbar drags with its title and chip ("deep": buttons inside still click), and so does the
    // sidebar's top band beside the traffic lights.
    const drag = find(m, (el) => el.props['data-tauri-drag-region'] !== undefined)
    expect(drag.map((el) => [el.props.className, el.props['data-tauri-drag-region']])).toEqual([['mw-drag-band', ''], ['mw-toolbar', 'deep']])
    expect(navOf(m)[0].props.children[0].props.className).toBe('mw-drag-band')
    expect(find(m, (el) => el.type === 'main')[0].props['aria-labelledby']).toBe('mw-title')
  })

  test('Home\'s toolbar carries the encryption state while the vault is unlocked (hi-fi section 3), and only on Home', async () => {
    const { m, request } = openWindow()
    await settle(m)
    expect(find(m, (el) => el.props.className === 'mw-chip').map((el) => textOf(expand(el.props.children)).trim())).toEqual(['End-to-end encrypted'])
    await request({ to: 'settings', section: null })
    expect(find(m, (el) => el.props.className === 'mw-chip')).toHaveLength(0)
  })

  test('a locked vault says so in the footer, not a plan, and claims no encryption chip', async () => {
    const { m } = openWindow({ snap: () => snapshot({ account: { vault_unlocked: false } }) })
    await settle(m)
    expect(find(m, (el) => el.props.className === 'mw-chip')).toHaveLength(0)
    expect(textOf(expand(m.tree()))).toContain('Vault locked')
    expect(m.calls.filter((c) => c.name === 'account_subscription')).toHaveLength(0)
  })
})

describe('Settings in System Settings style (ruling 1, §7.8)', () => {
  test('the gear swaps the sidebar for the Settings list: back to Home, the heading, the account row, General, Sync, About', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    const nav = navOf(m)
    expect(nav.map((n) => n.props['aria-label'])).toEqual(['Settings'])
    expect(buttonsIn(nav[0]).map(label)).toEqual(['Back to Home', 'Account, sam@example.eu, Basic plan', 'General', 'Sync', 'About'])
    expect(textOf(expand(nav[0].props.children))).toContain('Settings')
    expect(current(m)).toEqual(['General'])
    expect(shown(m)).toEqual(['general'])
    expect(title(m)).toBe('General')
    // One window: the same root, the same drag region; no second sidebar.
    expect(navOf(m)).toHaveLength(1)
    expect(focused.at(-1)).toBe('mw-settings-general')
    // The Settings list keeps the drag band above its back control.
    expect(nav[0].props.children[0].props.className).toBe('mw-drag-band')
    expect(nav[0].props.children[0].props['data-tauri-drag-region']).toBe('')
  })

  test('the account row is the Account section; exactly one section body is mounted at a time', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    await click(m, 'Account, sam@example.eu, Basic plan')
    expect(shown(m)).toEqual(['account'])
    expect(title(m)).toBe('Account')
    await click(m, 'About')
    expect(shown(m)).toEqual(['about'])
  })

  test('back returns to Home and puts focus on the gear (§11)', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    await click(m, 'Back to Home')
    expect(navOf(m).map((n) => n.props['aria-label'])).toEqual(['Beebeeb'])
    expect(shown(m)).toEqual(['home'])
    expect(focused.at(-1)).toBe('mw-gear')
  })

  test('↑ and ↓ move and select in the Settings list, Home and End jump; ← and → are left alone', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    // Every item takes the list's keys (one Tab stop, roving tabindex); press them on the selected one, as focus is.
    const selected = () => find(m, (el) => el.type === 'button' && el.props['aria-current'] === 'page')[0]
    let prevented = 0
    const key = async (k: string) => { selected().props.onKeyDown({ key: k, preventDefault: () => { prevented += 1 } }); m.render(); await settle(m) }
    await key('ArrowDown')
    expect(shown(m)).toEqual(['sync'])
    expect(focused.at(-1)).toBe('mw-settings-sync')
    await key('End')
    expect(shown(m)).toEqual(['about'])
    await key('ArrowDown')
    expect(shown(m)).toEqual(['account'])
    await key('Home')
    expect(shown(m)).toEqual(['account'])
    await key('ArrowUp')
    expect(shown(m)).toEqual(['about'])
    await key('ArrowRight')
    await key('ArrowLeft')
    expect(shown(m)).toEqual(['about'])
    expect(prevented).toBe(5)
  })

  test('the list is one Tab stop: only the selected section has tabIndex 0', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    await click(m, 'Sync')
    expect(buttonsIn(navOf(m)[0]).slice(1).map((b) => b.props.tabIndex)).toEqual([-1, -1, 0, -1])
  })

  test('⌘, opens the last section used in this window', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Settings')
    await click(m, 'Sync')
    await click(m, 'Back to Home')
    await click(m, 'Settings')
    expect(shown(m)).toEqual(['sync'])
  })
})

describe('one request channel: main:navigate (§7.1)', () => {
  test('the window pulls a pending request as soon as it listens (a window built for it), then on every poke', async () => {
    let pending: unknown = { to: 'settings', section: 'about' }
    const { m, bus, request } = openWindow({ backend: { consume_main_navigation: () => { const nav = pending; pending = null; return nav } } })
    await settle(m)
    expect(bus.names()).toContain(windowModel.MAIN_NAVIGATE_EVENT)
    expect(shown(m)).toEqual(['about'])
    pending = { to: 'home' }
    await request(pending)
    expect(shown(m)).toEqual(['home'])
  })

  test('⌘1 from Settings goes Home; ⌘[ goes back; ⌘, with no section opens General', async () => {
    const { m, request } = openWindow()
    await settle(m)
    await request({ to: 'settings', section: null })
    expect(shown(m)).toEqual(['general'])
    await request({ to: 'home' })
    expect(shown(m)).toEqual(['home'])
    expect(focused.at(-1)).toBe('mw-section-home')
    await request({ to: 'settings', section: 'sync' })
    await request({ to: 'back' })
    expect(shown(m)).toEqual(['home'])
    expect(focused.at(-1)).toBe('mw-gear')
  })

  test('the window tells Rust whether Settings shows, so View › Back is enabled only then (F15)', async () => {
    const { m, request } = openWindow()
    await settle(m)
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: false, sheetOpen: false })
    await request({ to: 'settings', section: null })
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: true, sheetOpen: false })
    await request({ to: 'back' })
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: false, sheetOpen: false })
  })
})

describe('§8.2: nothing navigates while a sheet is open', () => {
  test('with a sheet open, ⌘1, ⌘[ and ⌘, change nothing, and Rust is told a sheet is open', async () => {
    let modals = 0
    const { m, request } = openWindow({ bindings: { openModalCount: () => modals } })
    await settle(m)
    await request({ to: 'settings', section: 'account' })
    expect(shown(m)).toEqual(['account'])
    modals = 1
    m.render()
    await settle(m)
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: true, sheetOpen: true })
    for (const nav of [{ to: 'home' }, { to: 'back' }, { to: 'settings', section: null }, { to: 'settings', section: 'general' }]) {
      await request(nav)
      expect(shown(m)).toEqual(['account'])
    }
    modals = 0
    m.render()
    await settle(m)
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: true, sheetOpen: false })
    await request({ to: 'home' })
    expect(shown(m)).toEqual(['home'])
  })

  test('the sign-out flow\'s own sheets count: during the confirm, a request to leave Account is ignored', async () => {
    let pendingFlow = true
    const { m, request } = openWindow({
      backend: {
        consume_sign_out_flow_request: () => { const was = pendingFlow; pendingFlow = false; return was },
        sign_out_count: () => ({ askId: 'a1', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: 'offline' }),
      },
      bindings: { connectSignOutFlowMenu },
    })
    await settle(m)
    // The flow request lands on Settings › Account and opens the ask.
    expect(shown(m)).toEqual(['account'])
    const account = find(m, (el) => el.props['data-tab'] === 'account')[0]
    expect(account.props['data-props'].flowView.kind).toBe('ask')
    await request({ to: 'home' })
    expect(shown(m)).toEqual(['account'])
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: true, sheetOpen: true })
  })
})

describe('moved from the Settings window\'s root (task 1683 slice 4, now the window shell)', () => {
  test('the window drains the native update-check request when it listens and on every menu event', async () => {
    let onMenu: () => void = () => {}
    const { m } = openWindow({
      bindings: {
        listen: (event: string, callback: () => void) => { if (event === 'menu:check-for-updates') onMenu = callback; return Promise.resolve(() => {}) },
        connectNativeUpdateMenu: realConnectNativeUpdateMenu,
      },
    })
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(1)
    onMenu()
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(2)
  })

  test('a consumed update-check lands on Settings › About; a plain open consumes nothing and stays on Home', async () => {
    const consumed = openWindow({ backend: { consume_menu_update_check: () => true }, bindings: { connectNativeUpdateMenu: realConnectNativeUpdateMenu } })
    await settle(consumed.m)
    expect(shown(consumed.m)).toEqual(['about'])
    const plain = openWindow({ bindings: { connectNativeUpdateMenu: realConnectNativeUpdateMenu } })
    await settle(plain.m)
    expect(plain.m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(1)
    expect(shown(plain.m)).toEqual(['home'])
  })

  /** F9: the flow and its request live in the shell. A request handled once never runs again, however often the
   *  Account section mounts: a request, Cancel at the ask, Sync, Account again → one count, no `clear_session`. */
  test('a handled flow request never runs again when the Account section mounts again', async () => {
    const source = readFileSync(new URL('../src/MacSettings.tsx', import.meta.url), 'utf8')
    const start = source.indexOf('function AccountTab(')
    expect(start, 'AccountTab is in MacSettings.tsx').toBeGreaterThan(-1)
    const tab = source.slice(start, source.indexOf('\n}\n', start))
    for (const banned of ['new SignOutFlow(', 'connectSignOutFlowMenu(', 'connectSignOutFlowEvents(', '.request()']) {
      expect(tab.includes(banned), banned).toBe(false)
    }
    let pending = true
    let flow: any = null
    const accountStub = (props: any) => { flow = props.flow; return createElement('div', { 'data-tab': 'account' }) }
    const { m } = openWindow({
      backend: {
        consume_sign_out_flow_request: () => { const was = pending; pending = false; return was },
        sign_out_count: () => ({ askId: 'a1', files: 1, copiedFiles: 1, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: 'offline' }),
        clear_session: () => ({ warning: null }),
      },
      bindings: { AccountTab: accountStub, connectSignOutFlowEvents, connectSignOutFlowMenu },
    })
    await settle(m)
    expect(shown(m)).toEqual(['account'])
    expect(flow.current().kind).toBe('ask')
    flow.cancel()
    m.render()
    await click(m, 'Sync')
    await click(m, 'Account, sam@example.eu, Basic plan')
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'sign_out_count')).toHaveLength(1)
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(0)
    expect(flow.current().kind).toBe('idle')
  })

  test('a flow request reaches Settings › Account even while another sheet is open (as the Settings root did: the flow is the one exception to §8.2)', async () => {
    let pending = false
    let modals = 0
    let onFlow: () => void = () => {}
    const { m, request } = openWindow({
      backend: {
        consume_sign_out_flow_request: () => { const was = pending; pending = false; return was },
        sign_out_count: () => ({ askId: 'a1', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }),
      },
      bindings: {
        connectSignOutFlowMenu,
        openModalCount: () => modals,
        listen: (event: string, callback: () => void) => { if (event === 'menu:sign-out-flow') onFlow = callback; return Promise.resolve(() => {}) },
      },
    })
    await settle(m)
    await request({ to: 'settings', section: 'sync' })
    modals = 1 // Repair's confirm, open in Sync
    m.render()
    await settle(m)
    pending = true
    onFlow()
    await settle(m)
    expect(shown(m)).toEqual(['account'])
  })

  test('a flow request from Home selects Settings › Account before the flow runs (entry points 2 and 5, §8.1)', async () => {
    let pending = true
    const order: string[] = []
    const { m } = openWindow({
      backend: {
        consume_sign_out_flow_request: () => { const was = pending; pending = false; return was },
        sign_out_count: () => { order.push('count'); return { askId: 'a1', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null } },
      },
      bindings: { connectSignOutFlowMenu },
    })
    await settle(m)
    expect(shown(m)).toEqual(['account'])
    expect(current(m)).toEqual(['Account, sam@example.eu, Basic plan'])
    expect(order).toEqual(['count'])
  })
})

describe('signed out (until slice 2 brings sign-in into the window)', () => {
  const out = () => snapshot({ phase: 'signed_out', account: { email: null, logged_in: false, vault_unlocked: false } })

  test('one centred card, no sidebar; its button opens today\'s sign-in', async () => {
    const { m } = openWindow({ snap: out, backend: { open_onboarding_window: () => null } })
    await settle(m)
    expect(navOf(m)).toHaveLength(0)
    const card = expand(m.tree())
    expect(textOf(card)).toContain('Sign in to Beebeeb')
    await byLabel(m, 'Sign in').props.onClick()
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'open_onboarding_window')).toHaveLength(1)
    expect(m.toasts).toEqual([])
  })

  test('⌘, shows Settings with General and About only, and the back control is "Sign in"', async () => {
    const { m, request } = openWindow({ snap: out })
    await settle(m)
    await request({ to: 'settings', section: 'account' })
    expect(buttonsIn(navOf(m)[0]).map(label)).toEqual(['Back to Sign in', 'General', 'About'])
    expect(shown(m)).toEqual(['general'])
    await click(m, 'Back to Sign in')
    expect(navOf(m)).toHaveLength(0)
    expect(textOf(expand(m.tree()))).toContain('Sign in to Beebeeb')
  })

  test('a held sign-out sentence is shown above the card, neutrally', async () => {
    accountSession.holdSignOutWarning('Beebeeb couldn’t confirm it was removed from Finder.')
    const { m } = openWindow({ snap: out })
    await settle(m)
    const text = textOf(expand(m.tree()))
    expect(text).toContain('Beebeeb couldn’t confirm it was removed from Finder.')
    expect(text.indexOf('couldn’t confirm')).toBeLessThan(text.indexOf('Sign in to Beebeeb'))
    expect(find(m, (el) => el.props.role === 'alert')).toEqual([])
  })

  test('an ended session is "Sign in again", names the account, and signs in again in place', async () => {
    const ended = () => snapshot({ phase: 'session_ended', account: { auth_expired: true } })
    const { m } = openWindow({ snap: ended, backend: { desktop_platform: () => 'macos', open_reauth_window: () => null } })
    await settle(m)
    const text = textOf(expand(m.tree()))
    expect(text).toContain('Sign in again')
    expect(text).toContain('Sync is paused until you sign in again. Your files stay in Finder.')
    expect(text).toContain('Signed in before as sam@example.eu')
    await byLabel(m, 'Sign in again').props.onClick()
    await settle(m)
    expect(m.calls.map((c) => c.name)).toContain('open_reauth_window')
    expect(m.calls.map((c) => c.name)).not.toContain('clear_session')
  })
})

// ── Home (§7.6, first cut) ──────────────────────────────────────────────────

function finderBus() {
  return {
    subscribeFinderSetup(_onView: unknown, options: { onSubscribed?: () => void } = {}) {
      void Promise.resolve().then(() => options.onSubscribed?.())
      return () => {}
    },
  }
}
const useFinderSetupModule = () => ({
  file: 'finderSetup.ts',
  name: 'useFinderSetup',
  bindings: {
    loadFinderSetup: finderSetup.loadFinderSetup,
    runFinderSetupAction: (action: finderSetupCopy.FinderSetupAction) => finderSetup.runFinderSetupAction(action, { writeClipboard: async () => {} }),
    finderSetupLoadPresentation: finderSetupCopy.finderSetupLoadPresentation,
    FINDER_ACTION_FAILED: finderSetupCopy.FINDER_ACTION_FAILED,
    FINDER_COPIED_MS: finderSetupCopy.FINDER_COPIED_MS,
    FINDER_ACTION_COMMAND: finderSetup.FINDER_ACTION_COMMAND,
    commandUnavailableLabel: desktopApi.commandUnavailableLabel,
    subscribeFinderSetup: finderBus().subscribeFinderSetup,
    loadEngineRefusal: desktopApi.loadEngineRefusal,
  },
})

function openHome(snap: unknown, backend: Record<string, (args: any) => unknown> = {}) {
  let refreshed = 0
  const m = mount('MacApp.tsx', 'HomePane', {
    expand: true,
    backend: {
      finder_setup_state: () => ({ setup: 'ready', reason: null, launch_location: 'applications', attempt: 1, max_attempts: 1, last_failure: null }),
      sync_status: () => ({ logged_in: true, engine: 'running', sync_root: null, syncing: 0, cloud_only: 0, conflicts: 0, engine_refusal: null }),
      kept_unsynced_folder: () => null,
      kept_unsent_folder: () => null,
      ...backend,
    } as any,
    props: { snapshot: snap, onRefresh: () => { refreshed += 1 } },
    hookModules: [useFinderSetupModule(), useKeptFoldersModule()],
    bindings: {
      ...desktopApi,
      ...model,
      ...parts,
      ...windowModel,
      finderActionButtonLabel: finderSetupCopy.finderActionButtonLabel,
      finderOpenFailedToast: finderSetup.finderOpenFailedToast,
      KeptFolderNotes,
    },
  })
  mounted.push(m)
  return { m, refreshed: () => refreshed }
}
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
const settleHome = async (m: Mounted) => { await m.flush(); await tick(); await m.flush(); await m.flush() }
const homeText = (m: Mounted) => {
  const out: string[] = []
  const walk = (n: any) => {
    if (Array.isArray(n)) return n.forEach(walk)
    if (n == null || typeof n === 'boolean') return
    if (typeof n !== 'object' || !n.props) { out.push(String(n)); return }
    if (n.type === 'svg') return
    walk(n.props.children)
  }
  walk(expand(m.tree()))
  return out.join(' | ')
}
const homeButton = (m: Mounted, text: string) => {
  const hit = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === text)
  if (hit.length !== 1) throw new Error(`expected one "${text}", found ${hit.length}`)
  return hit[0]
}

describe('Home (§7.6, first cut: status, Finder row, storage, the kept rows)', () => {
  test('up to date, Beebeeb in Finder with the one amber Open in Finder, and storage', async () => {
    const { m } = openHome(snapshot(), { open_finder_location: () => null })
    await settleHome(m)
    const text = homeText(m)
    expect(text).toContain('Up to date')
    expect(text).toContain('Last checked just now')
    expect(text).toContain('Beebeeb in Finder')
    expect(text).toContain('Your vault appears under Locations in Finder.')
    expect(text).toContain(model.storageLine(snapshot().storage)!.label)
    const open = homeButton(m, 'Open in Finder')
    expect(open.props.className).toContain('ms-btn--primary')
    const primaries = find(m, (el) => el.type === 'button' && String(el.props.className).includes('ms-btn--primary'))
    expect(primaries).toHaveLength(1)
    await open.props.onClick()
    await settleHome(m)
    expect(m.calls.find((c) => c.name === 'open_finder_location')?.args).toEqual({ path: null })
    expect(text).not.toContain('Needs your attention')
  })

  test('a failed Open in Finder is the one Finder sentence as a toast', async () => {
    const { m } = openHome(snapshot(), { open_finder_location: () => { throw new Error('-54') } })
    await settleHome(m)
    await homeButton(m, 'Open in Finder').props.onClick()
    await settleHome(m)
    expect(m.toasts).toEqual([finderSetup.finderOpenFailedToast()])
  })

  test('the kept rows show under "Needs your attention" with the words and buttons of Settings › Sync (§8.3)', async () => {
    const ours = '/Users/sam/Library/Containers/io.beebeeb.app/Data/Documents/Changes not sent/2026-10-10 at 14.05.12'
    const macos = '/Users/sam/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 14.05)'
    let unsent: string | null = ours
    const { m } = openHome(snapshot(), {
      kept_unsent_folder: () => unsent,
      kept_unsynced_folder: () => macos,
      dismiss_kept_unsent_folder: (args: any) => { const was = unsent; unsent = null; return { cleared: args.path === was, current: null } },
    })
    await settleHome(m)
    const text = homeText(m)
    expect(text).toContain('Needs your attention')
    expect(text).toContain(signOutCopy.K2_ROW)
    expect(text).toContain(ours)
    expect(text).toContain(model.KEPT_FOLDER_ROW_SENTENCE)
    expect(text.indexOf(signOutCopy.K2_ROW)).toBeLessThan(text.indexOf(model.KEPT_FOLDER_ROW_SENTENCE))
    const dismiss = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === signOutCopy.K2_DISMISS)
    await dismiss[0].props.onClick()
    await settleHome(m)
    expect(m.calls.find((c) => c.name === 'dismiss_kept_unsent_folder')?.args).toEqual({ path: ours })
    expect(homeText(m)).not.toContain(signOutCopy.K2_ROW)
  })

  test('paused says how many changes wait, with Resume sync; offline and error never offer an action that does not exist', async () => {
    const paused = openHome(snapshot({ phase: 'paused', pending_changes: 3 }), { tray_resume_sync: () => null })
    await settleHome(paused.m)
    expect(homeText(paused.m)).toContain('3 changes on this Mac are waiting. Nothing syncs until you resume.')
    await homeButton(paused.m, 'Resume sync').props.onClick()
    await settleHome(paused.m)
    expect(paused.m.calls.filter((c) => c.name === 'tray_resume_sync')).toHaveLength(1)
    expect(paused.refreshed()).toBe(1)
    const offline = openHome(snapshot({ phase: 'offline' }))
    await settleHome(offline.m)
    expect(homeText(offline.m)).toContain('Can’t reach Beebeeb')
    expect(find(offline.m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim())).toEqual(['Open in Finder', 'Add storage'])
  })

  test('locked: one card with the popover\'s sentence and Unlock vault, nothing else', async () => {
    const { m, refreshed } = openHome(snapshot({ phase: 'locked', account: { vault_unlocked: false } }), { unlock_vault: () => null })
    await settleHome(m)
    const text = homeText(m)
    expect(text).toContain('Your vault is locked')
    expect(text).toContain('Sync is paused until you unlock it. Files already downloaded to this Mac stay where they are.')
    expect(text).not.toContain('Beebeeb in Finder')
    await homeButton(m, 'Unlock vault').props.onClick()
    await settleHome(m)
    expect(m.calls.filter((c) => c.name === 'unlock_vault')).toHaveLength(1)
    expect(refreshed()).toBe(1)
  })

  test('a Finder failure is one inline notice under the row, never a toast (one failure, one surface)', async () => {
    const { m } = openHome(snapshot({ phase: 'finder_failed' }), {
      finder_setup_state: () => ({ setup: 'failed', reason: 'timeout', launch_location: 'applications', attempt: 4, max_attempts: 4, last_failure: null }),
    })
    await settleHome(m)
    const text = homeText(m)
    expect(text).toContain('macOS didn’t finish adding Beebeeb to Finder.')
    expect(find(m, (el) => el.props.role === 'alert')).toHaveLength(1)
    expect(m.toasts).toEqual([])
    expect(text).not.toContain('Open in Finder')
  })
})


// ── Review and Trash (task 1683 slice 4; spec §7.6, §9.3, §8.2, §11) ─────────────────────────────────────────────

const entryFixture = (id: string) => ({ id, file_id: `f-${id}`, file_name: `${id}.md`, kind: 'conflict', status: 'needs review', updated_at: 1000, detail: 'd', action: 'open_conflict' })
const readyFeed = (n: number) => ({ status: 'ready' as const, entries: Array.from({ length: n }, (_, i) => entryFixture(`e${i}`)), reason: null })
const badgeOf = (m: Mounted) => find(m, (el) => el.props.className === 'mw-badge').map((el) => textOf(expand(el.props.children)).trim())
const paneProps = (m: Mounted, tab: string) => find(m, (el) => el.props['data-tab'] === tab)[0]?.props['data-props']

describe('the sidebar lists Home, Review and Trash (§7.6)', () => {
  test('three rows in this order, each with its own id, the selected one current, one Tab stop', async () => {
    const { m } = openWindow()
    await settle(m)
    const list = buttonsIn(navOf(m)[0]).filter((b) => b.props.id !== 'mw-gear')
    expect(list.map(label)).toEqual(['Home', 'Review', 'Trash'])
    expect(list.map((b) => b.props.id)).toEqual(['mw-section-home', 'mw-section-review', 'mw-section-trash'])
    expect(list.map((b) => b.props.tabIndex)).toEqual([0, -1, -1])
  })

  test('Review carries the number of things waiting, only when there are some and the vault is unlocked', async () => {
    const some = openWindow({ feed: readyFeed(2) })
    await settle(some.m)
    expect(badgeOf(some.m)).toEqual(['2'])
    const review = find(some.m, (el) => el.type === 'button' && el.props.id === 'mw-section-review')[0]
    expect(review.props['aria-label']).toBe('Review, 2 need attention')
    const none = openWindow({ feed: readyFeed(0) })
    await settle(none.m)
    expect(badgeOf(none.m)).toEqual([])
    const locked = openWindow({ feed: readyFeed(2), snap: () => snapshot({ account: { vault_unlocked: false } }) })
    await settle(locked.m)
    expect(badgeOf(locked.m)).toEqual([])
  })

  test('a single item says "1 needs attention", not "1 need"', async () => {
    const { m } = openWindow({ feed: readyFeed(1) })
    await settle(m)
    expect(find(m, (el) => el.props.id === 'mw-section-review')[0].props['aria-label']).toBe('Review, 1 needs attention')
  })

  test('a feed that is still loading or failed shows no number: no placeholder, no guess', async () => {
    for (const status of ['loading', 'failed'] as const) {
      const { m } = openWindow({ feed: { status, entries: [], reason: null } })
      await settle(m)
      expect(badgeOf(m)).toEqual([])
    }
  })
})

describe('⌘2 and ⌘3, the sidebar and the keys (§9.3, §11)', () => {
  test('a request for Review shows Review: the pane, the title, the current row and the focus', async () => {
    const { m, request } = openWindow({ feed: readyFeed(1) })
    await settle(m)
    await request({ to: 'review' })
    expect(shown(m)).toEqual(['review'])
    expect(title(m)).toBe('Review')
    expect(current(m)).toEqual(['Review, 1 needs attention'])
    expect(focused.at(-1)).toBe('mw-section-review')
    await request({ to: 'trash' })
    expect(shown(m)).toEqual(['trash'])
    expect(title(m)).toBe('Trash')
    expect(focused.at(-1)).toBe('mw-section-trash')
  })

  test('clicking a row shows its pane', async () => {
    const { m } = openWindow()
    await settle(m)
    await click(m, 'Review')
    expect(shown(m)).toEqual(['review'])
    await click(m, 'Trash')
    expect(shown(m)).toEqual(['trash'])
    await click(m, 'Home')
    expect(shown(m)).toEqual(['home'])
  })

  test('↓ and ↑ move through Home, Review, Trash and wrap; Home and End jump; nothing else is taken', async () => {
    const { m } = openWindow()
    await settle(m)
    const selected = () => find(m, (el) => el.type === 'button' && el.props['aria-current'] === 'page')[0]
    let prevented = 0
    const key = async (k: string) => { selected().props.onKeyDown({ key: k, preventDefault: () => { prevented += 1 } }); m.render(); await settle(m) }
    await key('ArrowDown')
    expect(shown(m)).toEqual(['review'])
    expect(focused.at(-1)).toBe('mw-section-review')
    await key('ArrowDown')
    expect(shown(m)).toEqual(['trash'])
    await key('ArrowDown')
    expect(shown(m)).toEqual(['home'])
    await key('ArrowUp')
    expect(shown(m)).toEqual(['trash'])
    await key('Home')
    expect(shown(m)).toEqual(['home'])
    await key('End')
    expect(shown(m)).toEqual(['trash'])
    await key('ArrowLeft')
    expect(prevented).toBe(6)
  })

  test('Settings entered from Review comes back to Review: the back control names it, and ⌘[ goes there', async () => {
    const { m, request } = openWindow()
    await settle(m)
    await request({ to: 'review' })
    await click(m, 'Settings')
    expect(buttonsIn(navOf(m)[0]).map(label)[0]).toBe('Back to Review')
    await request({ to: 'back' })
    expect(shown(m)).toEqual(['review'])
    expect(focused.at(-1)).toBe('mw-gear')
  })

  test('⌘2 from Settings leaves it for Review', async () => {
    const { m, request } = openWindow()
    await settle(m)
    await request({ to: 'settings', section: 'sync' })
    await request({ to: 'review' })
    expect(navOf(m).map((n) => n.props['aria-label'])).toEqual(['Beebeeb'])
    expect(shown(m)).toEqual(['review'])
  })

  test('§8.2: with a sheet open over Review (the conflict sheet), ⌘1, ⌘3 and ⌘, change nothing, and Rust is told', async () => {
    let modals = 0
    const { m, request } = openWindow({ bindings: { openModalCount: () => modals } })
    await settle(m)
    await request({ to: 'review' })
    modals = 1
    m.render()
    await settle(m)
    expect(menuStates(m).at(-1)).toEqual({ settingsShowing: false, sheetOpen: true })
    for (const nav of [{ to: 'home' }, { to: 'trash' }, { to: 'settings', section: null }]) {
      await request(nav)
      expect(shown(m)).toEqual(['review'])
    }
  })
})

describe('Review and Trash need the key (§7.6: "Review and Trash show the same card")', () => {
  test('locked, Review and Trash both show the locked card instead of their pane, and no toolbar action', async () => {
    const { m, request } = openWindow({ snap: () => snapshot({ phase: 'locked', account: { vault_unlocked: false } }) })
    await settle(m)
    await request({ to: 'review' })
    expect(shown(m)).toEqual(['locked'])
    expect(title(m)).toBe('Review')
    expect(find(m, (el) => el.type === 'button' && ['Version history…', 'Refresh'].includes(label(el)))).toHaveLength(0)
    await request({ to: 'trash' })
    expect(shown(m)).toEqual(['locked'])
    expect(title(m)).toBe('Trash')
  })

  test('the card asks the window to read the account again after an unlock attempt', async () => {
    const { m, request } = openWindow({ snap: () => snapshot({ account: { vault_unlocked: false } }) })
    await settle(m)
    await request({ to: 'review' })
    const before = m.calls.filter((c) => c.name === 'popover_snapshot').length
    paneProps(m, 'locked').onRefresh()
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'popover_snapshot').length).toBeGreaterThan(before)
  })
})

describe('the Review feed is read only while there is something to read (§7.6)', () => {
  test('signed in with an unlocked vault: connected once, and disconnected when the window goes', async () => {
    const { m, feedLog } = openWindow()
    await settle(m)
    expect(feedLog.connected).toBe(1)
    m.unmount()
    expect(feedLog.disconnected).toBe(1)
  })

  test('locked or signed out: never connected, so another account\'s or a locked vault\'s rows are never read', async () => {
    const locked = openWindow({ snap: () => snapshot({ account: { vault_unlocked: false } }) })
    await settle(locked.m)
    expect(locked.feedLog.connected).toBe(0)
    const out = openWindow({ snap: () => snapshot({ account: { logged_in: false, vault_unlocked: false } }) })
    await settle(out.m)
    expect(out.feedLog.connected).toBe(0)
  })

  test('Review is given the feed, and a resolved conflict makes the window read it again', async () => {
    const feed = readyFeed(2)
    const { m, request, feedLog } = openWindow({ feed })
    await settle(m)
    await request({ to: 'review' })
    const props = paneProps(m, 'review')
    expect(props.feed).toBe(feed)
    props.onChanged()
    expect(feedLog.refreshed).toBe(1)
  })
})

describe('the toolbar actions are the window\'s, not the panes\' (hi-fi section 3)', () => {
  test('the encryption chip is Home\'s alone: Review and Trash draw their buttons in that place, as the hi-fi does', async () => {
    const { m, request } = openWindow()
    await settle(m)
    const chips = () => find(m, (el) => el.props.className === 'mw-chip').length
    expect(chips()).toBe(1)
    await request({ to: 'review' })
    expect(chips()).toBe(0)
    await request({ to: 'trash' })
    expect(chips()).toBe(0)
  })

  test('Review has "Version history…" at the right of the toolbar; pressing it opens the sheet over Review, and the sheet closing closes it', async () => {
    const { m, request } = openWindow()
    await settle(m)
    await request({ to: 'review' })
    expect(paneProps(m, 'review').historyOpen).toBe(false)
    await click(m, 'Version history…')
    expect(paneProps(m, 'review').historyOpen).toBe(true)
    paneProps(m, 'review').onHistoryClose()
    m.render()
    await settle(m)
    expect(paneProps(m, 'review').historyOpen).toBe(false)
  })

  test('the history sheet is Review\'s: when the sign-out flow forces the window to Settings, it is closed when Review comes back', async () => {
    let pendingFlow = false
    const { m, bus, request } = openWindow({
      backend: {
        consume_sign_out_flow_request: () => { const was = pendingFlow; pendingFlow = false; return was },
        sign_out_count: () => ({ askId: 'a1', files: 0, copiedFiles: 0, others: 0, missingFiles: 0, earlierFolders: [], canSend: false, notSending: null }),
      },
      bindings: { connectSignOutFlowMenu },
    })
    await settle(m)
    await request({ to: 'review' })
    await click(m, 'Version history…')
    expect(paneProps(m, 'review').historyOpen).toBe(true)
    pendingFlow = true
    bus.emit('menu:sign-out-flow')
    await settle(m)
    expect(shown(m)).toEqual(['account'])
    await click(m, 'Back to Review')
    await settle(m)
    expect(shown(m)).toEqual(['review'])
    expect(paneProps(m, 'review').historyOpen).toBe(false)
  })

  test('Trash has Refresh; each press hands the pane a new signal so it loads again', async () => {
    const { m, request } = openWindow()
    await settle(m)
    await request({ to: 'trash' })
    const first = paneProps(m, 'trash').refreshSignal
    await click(m, 'Refresh')
    const second = paneProps(m, 'trash').refreshSignal
    await click(m, 'Refresh')
    const third = paneProps(m, 'trash').refreshSignal
    expect(new Set([first, second, third]).size).toBe(3)
  })

  test('Home, Settings and the signed-out card have neither, and neither button is the amber primary', async () => {
    const { m, request } = openWindow()
    await settle(m)
    const toolbarButtons = () => find(m, (el) => el.type === 'button' && ['Version history…', 'Refresh'].includes(label(el)))
    expect(toolbarButtons()).toHaveLength(0)
    await request({ to: 'settings', section: 'about' })
    expect(toolbarButtons()).toHaveLength(0)
    await request({ to: 'review' })
    expect(toolbarButtons().map((b) => String(b.props.className))).toEqual(['ms-btn'])
    await request({ to: 'trash' })
    expect(toolbarButtons().map((b) => String(b.props.className))).toEqual(['ms-btn'])
  })
})
