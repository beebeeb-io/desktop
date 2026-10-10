/**
 * The macOS Settings window's behaviour (task 1683 slice 4): the real component functions of
 * `src/MacSettings.tsx`, executed with the component harness against a scripted backend.
 *
 * What this proves: the handlers, the commands they send and the text a user would read, for a
 * scripted sequence of answers; "one failure, one surface" (spec section 7), "nothing destructive
 * without a second click", and that the support-bundle copy is the exact copy task 1685 pinned.
 * What it does NOT prove: layout (tests/render-mac-settings.mjs measures that in Chromium),
 * React scheduling, focus, or anything on a Mac. Mutations that went red are in the task Notes.
 */
import { afterEach, describe, expect, test } from 'bun:test'
import { createElement, Fragment } from 'react'
import * as desktopApi from '../src/desktopApi'
import { connectNativeUpdateMenu as realConnectNativeUpdateMenu, desktopUpdateCheck } from '../src/windows/manualUpdateCheck'
import * as diagnosticsCopy from '../src/diagnosticsCopy'
import * as finderInstallCard from '../src/finderInstallCard'
import * as model from '../src/macSettingsModel'
import * as parts from '../src/macSettingsParts'
import { expand, loadComponent, mount, textOf, visibleErrorSurfaces, type Mounted, type TreeNode } from './fixtures/componentHarness'

const React = { createElement, Fragment }

/** The in-window dialog, reduced to what a user sees: nothing while closed, title, body and footer while open. */
function Modal(props: any) {
  return props.open
    ? createElement('div', { role: 'dialog', 'aria-label': props.title }, createElement('h2', null, props.title), props.children, props.footer)
    : null
}

const ConfirmSheet = loadComponent('MacSettings.tsx', 'ConfirmSheet', { React, Btn: parts.Btn, Modal })
const ConfigLoadFailed = loadComponent('MacSettings.tsx', 'ConfigLoadFailed', { React, SettingsGroup: parts.SettingsGroup, Note: parts.Note, Btn: parts.Btn })
const withPinned = loadComponent('MacSettings.tsx', 'withPinned', {})

const baseBindings = {
  ...desktopApi,
  ...diagnosticsCopy,
  ...finderInstallCard,
  ...model,
  ...parts,
  Modal,
  ConfirmSheet,
  ConfigLoadFailed,
  withPinned,
  // The Mac's number format is pinned so the label does not depend on the machine running the test.
  storageLine: (storage: any) => model.storageLine(storage, 'nl-NL'),
}

const mounted: Mounted[] = []
afterEach(() => { while (mounted.length) mounted.pop()!.close() })

function open(name: string, backend: Record<string, (args: any) => unknown>, extra: { props?: any; bindings?: Record<string, unknown> } = {}) {
  const m = mount('MacSettings.tsx', name, {
    backend: backend as any,
    expand: true,
    props: extra.props,
    bindings: { ...baseBindings, ...extra.bindings },
  })
  mounted.push(m)
  return m
}

// ── helpers that read the rendered tree the way a user would ────────────────

function find(m: Mounted, match: (el: TreeNode) => boolean): TreeNode[] {
  return m.elements().filter(match)
}
const buttons = (m: Mounted) => find(m, (el) => el.type === 'button').map((el) => textOf(el.props.children).trim()).filter(Boolean)
const button = (m: Mounted, label: string) => {
  const hit = find(m, (el) => el.type === 'button' && textOf(el.props.children).trim() === label)
  if (hit.length !== 1) throw new Error(`expected one button "${label}", found ${hit.length}; buttons: ${buttons(m).join(' | ')}`)
  return hit[0]
}
const press = async (m: Mounted, label: string) => { await button(m, label).props.onClick(); await m.flush() }
/** Effects that depend on state set by an earlier effect need a render between them, as React does. */
const settle = async (m: Mounted) => { await m.flush(); await m.flush(); await m.flush() }
const dialogs = (m: Mounted) => find(m, (el) => el.props.role === 'dialog')
/** Everything a user would read: all text in the expanded tree except the options inside a select. */
function readable(node: any): string[] {
  if (Array.isArray(node)) return node.flatMap(readable)
  if (node == null || typeof node === 'boolean') return []
  if (typeof node !== 'object' || !node.props) return [String(node)]
  if (node.type === 'select' || node.type === 'svg') return []
  return readable(node.props.children)
}
const visibleText = (m: Mounted) => readable(expand(m.tree())).join(' | ')
const switchOf = (m: Mounted, name: string) => {
  const hit = find(m, (el) => el.props.role === 'switch' && el.props['aria-labelledby'] === `ms-${name}-label`)
  if (hit.length !== 1) throw new Error(`expected one switch "${name}", found ${hit.length}`)
  return hit[0]
}
const alerts = (m: Mounted) => find(m, (el) => el.props.role === 'alert')
const statuses = (m: Mounted) => find(m, (el) => el.props.role === 'status')

// ── fixtures ────────────────────────────────────────────────────────────────

const TIMEOUT_TEXT = 'Timed out waiting for the Beebeeb File Provider domain to become available'
const finder = {
  installed: { installed: true, path: 'Beebeeb in Finder', status: 'installed', last_error: null, last_attempt_at: 3, reason_category: null },
  missing: { installed: false, path: null, status: 'missing', last_error: null, last_attempt_at: null, reason_category: null },
  failed: { installed: false, path: null, status: 'error', last_error: TIMEOUT_TEXT, last_attempt_at: 2, reason_category: 'timeout' },
  userDisabled: { installed: false, path: null, status: 'error', last_error: 'Beebeeb is turned off in System Settings. Open Login Items & Extensions, then try again.', last_attempt_at: 4, reason_category: 'user_disabled' },
}
const config = { upload_kbps_limit: 0, download_kbps_limit: 5000, pause_sync: false, notify_conflicts: true, notify_sync_complete: false, notify_quota_warnings: true, theme: 'dark', local_cache_limit_bytes: 123 }
const snapshot = (over: { account?: object; storage?: object | null; phase?: string } = {}) => ({
  phase: over.phase ?? 'synced',
  generated_at: 1000,
  account: { email: 'sam@example.eu', logged_in: true, vault_unlocked: true, auth_expired: false, ...over.account },
  paused: false,
  engine: { state: 'idle', files_remaining: 0, bytes_total: 0, bytes_done: 0, last_tick_ok_at: 990 },
  reason: null,
  finder: { setup: 'ready', reason: null, reason_line: null },
  storage: over.storage === undefined ? { used_bytes: 84_300_000_000, quota_bytes: 200_000_000_000, fetched_at: 900, stale: false } : over.storage,
  storage_full: false,
  pending_changes: 0,
  conflicts: { count: 0, files: [] },
  activity: [],
})
const subscription = { plan: 'basic', billing_cycle: 'yearly', status: 'active', seats: 1, region: 'eu', current_period_end: '2026-10-14T12:00:00Z', quota_bytes: 1, used_bytes: 0 }

const folder = (id: string, name: string, pinned: boolean | undefined, children: any[] = []) => ({
  id, name, is_folder: true, excluded: false, size_bytes: 0, file_count: 0, on_disk_bytes: 0, children, ...(pinned === undefined ? {} : { pinned }),
})

// ── the shared config hook ──────────────────────────────────────────────────

describe('useSettingsConfig', () => {
  function probe(disk: { config: any; failWrites?: number; failReads?: number }) {
    const writes: any[] = []
    const m = open('useSettingsConfig', {
      get_desktop_config: () => {
        if (disk.failReads && disk.failReads > 0) { disk.failReads -= 1; throw new Error('read failed') }
        return structuredClone(disk.config)
      },
      set_desktop_config: ({ config: written }: any) => {
        writes.push(structuredClone(written))
        if (disk.failWrites && disk.failWrites > 0) { disk.failWrites -= 1; throw new Error('disk full') }
        disk.config = structuredClone(written)
      },
    })
    return { m, writes, hook: () => m.tree() as any }
  }

  test('reads the config once and is ready', async () => {
    const { m, hook } = probe({ config })
    expect(hook().state).toEqual({ status: 'loading' })
    await m.flush()
    expect(hook().state).toEqual({ status: 'ready', config })
    expect(m.calls.filter((c) => c.name === 'get_desktop_config')).toHaveLength(1)
  })

  test('a change writes the WHOLE config (fields this window never shows included), not just the changed key', async () => {
    const { m, writes, hook } = probe({ config })
    await m.flush()
    await hook().save({ notify_sync_complete: true })
    await m.flush()
    expect(writes).toHaveLength(1)
    expect(writes[0]).toEqual({ ...config, notify_sync_complete: true })
    expect(writes[0].theme).toBe('dark')
    expect(writes[0].local_cache_limit_bytes).toBe(123)
    expect(hook().state.config.notify_sync_complete).toBe(true)
  })

  test('two quick changes are written in order and the second carries the first', async () => {
    const { m, writes, hook } = probe({ config })
    await m.flush()
    const first = hook().save({ upload_kbps_limit: 2000 })
    const second = hook().save({ notify_conflicts: false })
    await Promise.all([first, second])
    await m.flush()
    expect(writes).toHaveLength(2)
    expect(writes[1]).toEqual({ ...config, upload_kbps_limit: 2000, notify_conflicts: false })
    expect(hook().state.config).toEqual({ ...config, upload_kbps_limit: 2000, notify_conflicts: false })
  })

  test('writes never overlap: the second waits for the first, so a slow first write cannot land after the second', async () => {
    let inFlight = 0
    let overlapped = false
    let release!: () => void
    const firstDone = new Promise<void>((resolve) => { release = resolve })
    const order: string[] = []
    const m = open('useSettingsConfig', {
      get_desktop_config: () => structuredClone(config),
      set_desktop_config: async ({ config: written }: any) => {
        inFlight += 1
        if (inFlight > 1) overlapped = true
        order.push(`start:${written.upload_kbps_limit}:${written.notify_conflicts}`)
        if (order.length === 1) await firstDone
        order.push(`end:${written.upload_kbps_limit}:${written.notify_conflicts}`)
        inFlight -= 1
      },
    })
    await m.flush()
    const hook = () => m.tree() as any
    const first = hook().save({ upload_kbps_limit: 2000 })
    const second = hook().save({ notify_conflicts: false })
    await Promise.resolve()
    release()
    await Promise.all([first, second])
    expect(overlapped).toBe(false)
    // Each write carries the snapshot its own change produced: the first sends its change alone,
    // the second sends both, so the last one on disk is the final state whatever order the
    // clicks and the disk were in. (A write that read whatever was latest when it started would
    // put a later change into an earlier write — which is exactly what made a failed first
    // write discard the second one, see the queued-write test above.)
    expect(order).toEqual(['start:2000:true', 'end:2000:true', 'start:2000:false', 'end:2000:false'])
  })

  test('a failed write is ONE toast and the window shows what is really saved, not what was clicked', async () => {
    const { m, writes, hook } = probe({ config, failWrites: 1 })
    await m.flush()
    await hook().save({ notify_sync_complete: true })
    await m.flush()
    expect(writes).toHaveLength(1)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t save that setting' })
    expect(hook().state).toEqual({ status: 'ready', config })
    expect(m.calls.filter((c) => c.name === 'get_desktop_config')).toHaveLength(2)
  })

  test('a later change survives a failed earlier queued write: it is still written, and the window says what is on disk', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const disk = { config: structuredClone(config) }
    const writes: any[] = []
    const m = open('useSettingsConfig', {
      get_desktop_config: () => structuredClone(disk.config),
      set_desktop_config: async ({ config: written }: any) => {
        writes.push(structuredClone(written))
        if (writes.length === 1) await gate
        if (writes.length === 1) throw new Error('disk full')
        disk.config = structuredClone(written)
      },
    })
    await m.flush()
    const hook = () => m.tree() as any
    const first = hook().save({ upload_kbps_limit: 2000 })
    const second = hook().save({ notify_conflicts: false })
    await Promise.resolve()
    release()
    await Promise.all([first, second])
    await m.flush()
    // The first write fails (one toast). The second change was queued behind it: it must still
    // reach the disk, computed from what is actually on disk — not be written over by the stale
    // config the failed write's read left behind, and not be reported saved when it was not.
    expect(m.toasts).toHaveLength(1)
    expect(writes).toHaveLength(2)
    expect(writes[1]).toEqual({ ...config, notify_conflicts: false })
    expect(disk.config).toEqual({ ...config, notify_conflicts: false })
    expect(hook().state).toEqual({ status: 'ready', config: { ...config, notify_conflicts: false } })
  })

  test('a load failure is its own state and Try again reads again', async () => {
    const { m, hook } = probe({ config, failReads: 1 })
    await m.flush()
    expect(hook().state).toEqual({ status: 'failed' })
    await hook().reload()
    await m.flush()
    expect(hook().state.status).toBe('ready')
  })
})

// ── General ─────────────────────────────────────────────────────────────────

describe('General tab', () => {
  const ready = (over: object = {}) => ({ state: { status: 'ready', config: { ...config, ...over } }, save: async () => {}, reload: async () => {} })

  test('shows the login switch and the three notification switches, with the spec\'s words, reflecting the saved config', async () => {
    const m = open('GeneralTab', { autostart_enabled: () => true }, { props: { settings: ready() } })
    await m.flush()
    const text = visibleText(m)
    for (const words of ['Open Beebeeb at login', 'Starts in the menu bar. No window opens.', 'Tell me about', 'Conflicts', 'When two versions of a file need a decision.', 'Sync finished', 'Local cache almost full']) {
      expect(text).toContain(words)
    }
    expect(switchOf(m, 'login').props['aria-checked']).toBe(true)
    expect(switchOf(m, 'notify_conflicts').props['aria-checked']).toBe(true)
    expect(switchOf(m, 'notify_sync_complete').props['aria-checked']).toBe(false)
    expect(switchOf(m, 'notify_quota_warnings').props['aria-checked']).toBe(true)
  })

  test('flipping a notification switch hands the window\'s config exactly that one key', async () => {
    const patches: any[] = []
    const settings = { ...ready(), save: async (patch: any) => { patches.push(patch) } }
    const m = open('GeneralTab', { autostart_enabled: () => true }, { props: { settings } })
    await m.flush()
    await switchOf(m, 'notify_sync_complete').props.onClick()
    await switchOf(m, 'notify_conflicts').props.onClick()
    expect(patches).toEqual([{ notify_sync_complete: true }, { notify_conflicts: false }])
  })

  test('every control is a real switch with an accessible name, never a bare div', async () => {
    const m = open('GeneralTab', { autostart_enabled: () => true }, { props: { settings: ready() } })
    await m.flush()
    const switches = find(m, (el) => el.props.role === 'switch')
    expect(switches).toHaveLength(4)
    for (const el of switches) {
      expect(el.type).toBe('button')
      expect(String(el.props['aria-labelledby'])).toMatch(/^ms-.+-label$/)
    }
  })

  test('the login switch toggles through toggle_autostart and shows the answer', async () => {
    const m = open('GeneralTab', { autostart_enabled: () => true, toggle_autostart: () => false }, { props: { settings: ready() } })
    await m.flush()
    await switchOf(m, 'login').props.onClick()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'toggle_autostart')).toHaveLength(1)
    expect(switchOf(m, 'login').props['aria-checked']).toBe(false)
    expect(m.toasts).toEqual([])
  })

  test('a login switch that fails is one toast and the switch stays where it was', async () => {
    const m = open('GeneralTab', { autostart_enabled: () => true, toggle_autostart: () => { throw new Error('not allowed') } }, { props: { settings: ready() } })
    await m.flush()
    await switchOf(m, 'login').props.onClick()
    await m.flush()
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t change the login setting' })
    expect(switchOf(m, 'login').props['aria-checked']).toBe(true)
  })

  test('a login setting that cannot be read disables its switch and says so, inline', async () => {
    const m = open('GeneralTab', { autostart_enabled: () => { throw new Error('nope') } }, { props: { settings: ready() } })
    await m.flush()
    expect(switchOf(m, 'login').props.disabled).toBe(true)
    expect(visibleText(m)).toContain('Couldn’t read this setting.')
    expect(m.toasts).toEqual([])
  })

  test('a config that failed to load is one inline alert with Try again, and no notification rows', async () => {
    let reloads = 0
    const settings = { state: { status: 'failed' }, save: async () => {}, reload: async () => { reloads += 1 } }
    const m = open('GeneralTab', { autostart_enabled: () => true }, { props: { settings } })
    await m.flush()
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(find(m, (el) => el.props['data-error-surface'] === 'settings-config')).toHaveLength(1)
    expect(find(m, (el) => el.props.role === 'switch')).toHaveLength(1) // only the login switch
    await press(m, 'Try again')
    expect(reloads).toBe(1)
  })
})

// ── Account ─────────────────────────────────────────────────────────────────

describe('Account tab', () => {
  const backend = (over: Record<string, (a: any) => unknown> = {}, snap = snapshot()) => ({
    popover_snapshot: () => snap,
    account_subscription: () => subscription,
    lock_vault: () => undefined,
    unlock_vault: () => undefined,
    clear_session: () => undefined,
    'plugin:opener|open_url': () => undefined,
    ...over,
  })

  test('shows who is signed in, the plan, one storage figure on the Mac\'s number format, and the vault', async () => {
    const m = open('AccountTab', backend())
    await settle(m)
    const text = visibleText(m)
    expect(text).toContain('sam@example.eu')
    expect(text).toContain('Basic plan · renews 14 Oct 2026')
    expect(text).toContain('84,3 GB of 200 GB')
    expect(text).toContain('Vault is unlocked')
    expect(text).toContain('Locking stops sync until you enter your password again.')
    expect(buttons(m)).toEqual(['Manage plan', 'Lock now', 'Sign out…'])
    const bar = find(m, (el) => el.props.role === 'progressbar')
    expect(bar).toHaveLength(1)
    expect(bar[0].props['aria-valuenow']).toBe(42)
    expect(bar[0].props['aria-label']).toBe('Storage used')
    expect(bar[0].props['data-red']).toBeUndefined()
  })

  test('there is one storage source: the snapshot, never a second fetch of the usage', async () => {
    const m = open('AccountTab', backend())
    await settle(m)
    expect(m.calls.map((c) => c.name).sort()).toEqual(['account_subscription', 'popover_snapshot'])
  })

  test('the storage bar turns red from 90 %', async () => {
    const m = open('AccountTab', backend({}, snapshot({ storage: { used_bytes: 190_000_000_000, quota_bytes: 200_000_000_000, fetched_at: 1, stale: false } })))
    await settle(m)
    expect(find(m, (el) => el.props.role === 'progressbar')[0].props['data-red']).toBe('true')
  })

  test('Lock now locks through lock_vault, reads the account again and shows the locked vault', async () => {
    let unlocked = true
    const m = open('AccountTab', backend({ lock_vault: () => { unlocked = false }, popover_snapshot: () => snapshot({ account: { vault_unlocked: unlocked }, storage: unlocked ? undefined : null }) }))
    await settle(m)
    await press(m, 'Lock now')
    expect(m.calls.filter((c) => c.name === 'lock_vault')).toHaveLength(1)
    expect(m.calls.filter((c) => c.name === 'popover_snapshot')).toHaveLength(2)
    expect(visibleText(m)).toContain('Vault is locked')
    expect(visibleText(m)).toContain('Sync is paused until you unlock it.')
    // Locked: the plan and Manage plan need the session, so they are gone, and Unlock is the one action.
    expect(buttons(m)).toEqual(['Unlock', 'Sign out…'])
    expect(button(m, 'Unlock').props.className).toContain('ms-btn--primary')
  })

  test('a failed lock is ONE toast and the vault still reads unlocked', async () => {
    const m = open('AccountTab', backend({ lock_vault: () => { throw new Error('busy') } }))
    await settle(m)
    await press(m, 'Lock now')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t lock the vault' })
    expect(visibleText(m)).toContain('Vault is unlocked')
    expect(visibleErrorSurfaces(m)).toEqual(['toast: Couldn’t lock the vault — busy'])
  })

  test('Sign out… asks first: nothing is sent until the second click, and Cancel sends nothing', async () => {
    const m = open('AccountTab', backend())
    await settle(m)
    expect(dialogs(m)).toHaveLength(0)
    await press(m, 'Sign out…')
    expect(dialogs(m)).toHaveLength(1)
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(0)
    expect(visibleText(m)).toContain('Sync stops until you sign in again.')
    await press(m, 'Cancel')
    expect(dialogs(m)).toHaveLength(0)
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(0)
    await press(m, 'Sign out…')
    await press(m, 'Sign out')
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(1)
    expect(dialogs(m)).toHaveLength(0)
  })

  test('after signing out the tab reads the account again and says you are signed out', async () => {
    let signedIn = true
    const m = open('AccountTab', backend({ clear_session: () => { signedIn = false }, popover_snapshot: () => snapshot({ account: { logged_in: signedIn, vault_unlocked: signedIn, email: signedIn ? 'sam@example.eu' : null }, storage: null }) }))
    await settle(m)
    await press(m, 'Sign out…')
    await press(m, 'Sign out')
    expect(visibleText(m)).toContain('You’re signed out')
    expect(buttons(m)).toEqual([])
  })

  test('a failed sign-out is one toast', async () => {
    const m = open('AccountTab', backend({ clear_session: () => { throw new Error('keychain locked') } }))
    await settle(m)
    await press(m, 'Sign out…')
    await press(m, 'Sign out')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ title: 'Couldn’t sign out' })
  })

  test('an account that cannot be read is ONE inline alert with Try again, not a toast', async () => {
    let fail = true
    const m = open('AccountTab', backend({ popover_snapshot: () => { if (fail) throw new Error('boom'); return snapshot() } }))
    await settle(m)
    expect(alerts(m)).toHaveLength(1)
    expect(find(m, (el) => el.props['data-error-surface'] === 'account-load')).toHaveLength(1)
    expect(m.toasts).toEqual([])
    fail = false
    await press(m, 'Try again')
    expect(visibleText(m)).toContain('sam@example.eu')
    expect(alerts(m)).toHaveLength(0)
  })

  test('a plan that cannot be read is said plainly on the plan line, and nothing else breaks', async () => {
    const m = open('AccountTab', backend({ account_subscription: () => { throw new Error('offline') } }))
    await settle(m)
    expect(visibleText(m)).toContain('Plan details aren’t available right now.')
    expect(visibleText(m)).toContain('84,3 GB of 200 GB')
    expect(m.toasts).toEqual([])
  })

  test('Manage plan opens the billing page', async () => {
    const m = open('AccountTab', backend())
    await settle(m)
    await press(m, 'Manage plan')
    expect(m.calls.find((c) => c.name === 'plugin:opener|open_url')?.args).toEqual({ url: desktopApi.BILLING_URL })
  })

  test('an ended session says so and offers no vault action', async () => {
    const m = open('AccountTab', backend({}, snapshot({ phase: 'session_ended', account: { auth_expired: true }, storage: null })))
    await settle(m)
    expect(visibleText(m)).toContain('Your session ended')
    expect(buttons(m)).not.toContain('Lock now')
    expect(buttons(m)).not.toContain('Unlock')
  })
})

// ── Sync ────────────────────────────────────────────────────────────────────

describe('Sync tab', () => {
  const ready = (over: object = {}) => ({ state: { status: 'ready', config: { ...config, ...over } }, save: async () => {}, reload: async () => {} })

  function syncBackend(opts: { finder?: any; install?: (a: any) => unknown; tree?: any[]; repair?: (a: any) => unknown; pin?: (a: any) => unknown; gate?: Promise<void>; kept?: string | null; dismissFails?: boolean; keptReadFails?: boolean; removesBeforeFailing?: boolean } = {}) {
    // `kept` is the folder Rust saved in desktop.toml (task 1882 round 2, review I2).
    const st: { finder: any; kept: string | null } = { finder: opts.finder ?? finder.installed, kept: opts.kept ?? null }
    return {
      st,
      backend: {
        finder_location_state: () => st.finder,
        install_finder_location: async (a: any) => {
          if (opts.gate) await opts.gate
          return opts.install ? opts.install(a) : st.finder
        },
        reset_macos_integration: (a: any) => {
          // A Repair that fails AFTER it removed the Finder location: the domain is already gone.
          if (opts.removesBeforeFailing) st.finder = finder.missing
          const result: any = opts.repair ? opts.repair(a) : { removed_file_provider_domain: true, disabled_autostart: true, removed_socket: true, removed_cache_files: 0, skipped_cache_files: 0, pending_operations_preserved: 0, warnings: [] }
          st.finder = finder.missing
          // Like Rust: a repair that kept files saves the folder for the row.
          if (typeof result?.preserved_location === 'string') st.kept = result.preserved_location
          return result
        },
        kept_unsynced_folder: () => {
          if (opts.keptReadFails) throw new Error('desktop.toml could not be read')
          return st.kept
        },
        dismiss_kept_unsynced_folder: (a: any) => {
          if (opts.dismissFails) throw new Error('disk full')
          if (st.kept === a?.path) st.kept = null
          return undefined
        },
        list_remote_tree: () => opts.tree ?? [folder('a', 'Photos', true), folder('b', 'Work', false)],
        set_recursive_pin: opts.pin ?? (() => undefined),
        open_login_items_and_extensions_settings: () => undefined,
      } as Record<string, (a: any) => unknown>,
    }
  }
  const openSync = async (opts: Parameters<typeof syncBackend>[0] = {}, settings: any = ready()) => {
    const { backend, st } = syncBackend(opts)
    const m = open('SyncTab', backend, { props: { settings } })
    await m.flush()
    return { m, st }
  }

  test('an added Finder location reads Added with Repair…, no error, and the spec\'s hint', async () => {
    const { m } = await openSync()
    const text = visibleText(m)
    expect(text).toContain('Beebeeb in Finder')
    expect(text).toContain('Your vault appears under Locations in Finder.')
    expect(text).toContain('Added')
    expect(buttons(m)).toContain('Repair…')
    expect(buttons(m)).not.toContain('Add to Finder')
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(visibleText(m)).not.toContain('Users')
  })

  test('a Finder location that is not added offers exactly one primary action, Add to Finder', async () => {
    const { m } = await openSync({ finder: finder.missing })
    expect(buttons(m)).toContain('Add to Finder')
    expect(buttons(m)).not.toContain('Repair…')
    expect(button(m, 'Add to Finder').props.className).toContain('ms-btn--primary')
    expect(visibleText(m)).toContain('Add it to see your files in Finder like any other folder.')
    expect(visibleText(m)).not.toContain('Your vault appears under Locations')
  })

  test('screenshot 1: a failed install is ONE inline alert (title, sentence, mono reason), never also a toast, and never the raw error', async () => {
    const { m } = await openSync({ finder: finder.missing, install: () => finder.failed })
    await press(m, 'Add to Finder')
    expect(find(m, (el) => el.props['data-error-surface'] === 'finder-install')).toHaveLength(1)
    expect(alerts(m)).toHaveLength(1)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
    const text = visibleText(m)
    expect(text).toContain('Couldn’t add Beebeeb to Finder')
    expect(text).toContain('macOS didn’t respond in time.')
    expect(text).toContain('reason: timeout')
    expect(text).not.toContain('File Provider')
    expect(text).not.toContain(TIMEOUT_TEXT)
    expect(buttons(m)).toContain('Try again')
  })

  test('the same failure that arrives as a rejected command is still one inline alert and no toast', async () => {
    const { m } = await openSync({ finder: finder.missing, install: () => { throw new Error('Finder location must be absolute: relative/path') } })
    await press(m, 'Add to Finder')
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
    expect(visibleText(m)).not.toContain('relative/path')
  })

  test('Add to Finder sends no path on macOS (the app picks its own location)', async () => {
    const { m } = await openSync({ finder: finder.missing, install: () => finder.installed })
    await press(m, 'Add to Finder')
    expect(m.calls.find((c) => c.name === 'install_finder_location')?.args).toEqual({ path: null })
    expect(buttons(m)).toContain('Repair…')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('a new attempt clears the old failure while it runs ("Adding…", disabled), then shows the new result once', async () => {
    let release!: () => void
    const gate = new Promise<void>((resolve) => { release = resolve })
    const { m } = await openSync({ finder: finder.failed, install: () => finder.failed, gate })
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    void button(m, 'Try again').props.onClick()
    m.render()
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(button(m, 'Adding…').props.disabled).toBe(true)
    release()
    await m.flush()
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
  })

  test('a user-disabled extension is a neutral notice with the System Settings action, not an error', async () => {
    const { m } = await openSync({ finder: finder.userDisabled })
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(alerts(m)).toHaveLength(0)
    expect(statuses(m)).toHaveLength(1)
    await press(m, 'Open Login Items & Extensions')
    expect(m.calls.filter((c) => c.name === 'open_login_items_and_extensions_settings')).toHaveLength(1)
    expect(m.toasts).toEqual([])
  })

  test('Repair… asks first, says it turns off Open Beebeeb at login, and sends nothing until the second click', async () => {
    const { m } = await openSync()
    await press(m, 'Repair…')
    expect(dialogs(m)).toHaveLength(1)
    expect(visibleText(m)).toContain('Repair Beebeeb in Finder?')
    expect(visibleText(m)).toContain('turns off Open Beebeeb at login')
    expect(m.calls.filter((c) => c.name === 'reset_macos_integration')).toHaveLength(0)
    await press(m, 'Cancel')
    expect(dialogs(m)).toHaveLength(0)
    expect(m.calls.filter((c) => c.name === 'reset_macos_integration')).toHaveLength(0)
  })

  test('confirming a repair resets once, reads the Finder state again and says what was kept, in one neutral line', async () => {
    const { m } = await openSync({ repair: () => ({ pending_operations_preserved: 3, warnings: [] }) })
    await press(m, 'Repair…')
    await press(m, 'Repair')
    expect(m.calls.filter((c) => c.name === 'reset_macos_integration')).toHaveLength(1)
    expect(m.calls.filter((c) => c.name === 'finder_location_state')).toHaveLength(2)
    expect(statuses(m).map((el) => textOf(el.props.children).trim())).toContain('3 changes waiting to upload were kept.')
    expect(buttons(m)).toContain('Add to Finder') // the repair removed it, so it can be added again
    expect(visibleErrorSurfaces(m)).toEqual([])
    expect(dialogs(m)).toHaveLength(0)
  })

  // Task 1882 (spec 2026-10-09 §5): the repair's removal kept files that never reached the server.
  const KEPT = '/Users/sam/Library/CloudStorage/Beebeeb (kept)'
  // Round 3 (re-review D3): the saved row outlives the account that kept the files, so its
  // sentence is neutral: no "your vault" for files that may belong to another account. The
  // literal is on purpose: it is what a person reads, not whatever the constant holds.
  const ROW_SENTENCE = 'Files that had not reached the server were kept in this folder:'
  const keptNotes = (m: Mounted) =>
    statuses(m).filter((el) => readable(expand(el)).join(' ').includes(ROW_SENTENCE))
  const monoLines = (m: Mounted) => find(m, (el) => String(el.props.className ?? '').split(' ').includes('ms-mono'))

  // Round 2 (review I2, lead ruling 2026-10-10): the kept folder is saved by Rust and shown as a
  // dismissible row until the person dismisses it. A tab switch or closing Settings unmounts
  // SyncTab, so the row must come from the saved folder, not from this component's state.
  test('a saved kept folder shows on open, in one status note whose path wraps, with Dismiss', async () => {
    const { m } = await openSync({ kept: KEPT })
    expect(keptNotes(m)).toHaveLength(1)
    const mono = monoLines(m)
    expect(mono.map((el) => textOf(el.props.children).trim())).toEqual([KEPT])
    expect(String(mono[0].props.className).split(' ')).toContain('ms-mono--wrap')
    expect(buttons(m)).toContain('Dismiss')
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  // Re-review D5: the fallback in runRepair covers a failed READ of the saved record, and only that.
  // A failed SAVE makes Repair return an error instead ("a repair that fails" below, one inline
  // alert and no row); the folder is then named by the app's own alert, which Rust raises before
  // it returns the error.
  test('a repair that kept files shows its folder even when the saved record cannot be read back', async () => {
    const { m } = await openSync({
      keptReadFails: true,
      repair: () => ({ pending_operations_preserved: 0, warnings: [], preserved_location: KEPT }),
    })
    expect(keptNotes(m)).toHaveLength(0) // nothing readable on open, so no row
    await press(m, 'Repair…')
    await press(m, 'Repair')
    expect(keptNotes(m)).toHaveLength(1)
    expect(monoLines(m).map((el) => textOf(el.props.children).trim())).toEqual([KEPT])
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('the saved row is neutral, so it reads correctly after an account switch (re-review D3)', async () => {
    const { m } = await openSync({ kept: KEPT })
    const [note] = keptNotes(m)
    const text = readable(expand(note)).join(' ')
    expect(text).toContain(ROW_SENTENCE)
    expect(text).not.toMatch(/\byour\b/i) // not "your vault": the files may be another account's
    expect(visibleText(m)).not.toContain(model.PRESERVED_FILES_SENTENCE)
    expect(model.KEPT_FOLDER_ROW_SENTENCE).toBe(ROW_SENTENCE)
  })

  test('the row survives a tab switch: a fresh Sync tab shows the same saved folder', async () => {
    const { backend, st } = syncBackend({ repair: () => ({ pending_operations_preserved: 0, warnings: [], preserved_location: KEPT }) })
    const first = open('SyncTab', backend, { props: { settings: ready() } })
    await first.flush()
    await press(first, 'Repair…')
    await press(first, 'Repair')
    expect(keptNotes(first)).toHaveLength(1)
    first.close()
    mounted.splice(mounted.indexOf(first), 1)
    expect(st.kept).toBe(KEPT)
    const again = open('SyncTab', backend, { props: { settings: ready() } })
    await again.flush()
    expect(keptNotes(again)).toHaveLength(1)
    expect(monoLines(again).map((el) => textOf(el.props.children).trim())).toEqual([KEPT])
  })

  test('Dismiss sends the exact folder the row showed and the row goes', async () => {
    const { m, st } = await openSync({ kept: KEPT })
    await press(m, 'Dismiss')
    expect(m.calls.filter((c) => c.name === 'dismiss_kept_unsynced_folder').map((c) => c.args)).toEqual([{ path: KEPT }])
    expect(st.kept).toBeNull()
    expect(keptNotes(m)).toHaveLength(0)
    expect(buttons(m)).not.toContain('Dismiss')
  })

  test('a Dismiss that fails keeps the row and says so once, in a toast', async () => {
    const { m } = await openSync({ kept: KEPT, dismissFails: true })
    await press(m, 'Dismiss')
    expect(keptNotes(m)).toHaveLength(1)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error' })
  })

  test('nothing saved shows no kept-folder row', async () => {
    const { m } = await openSync({ kept: null })
    expect(keptNotes(m)).toHaveLength(0)
    expect(buttons(m)).not.toContain('Dismiss')
  })

  test('a repair that kept un-synced files says so in one status note, with the folder in mono', async () => {
    const { m } = await openSync({ repair: () => ({ pending_operations_preserved: 0, warnings: [], preserved_location: KEPT }) })
    await press(m, 'Repair…')
    await press(m, 'Repair')
    expect(keptNotes(m)).toHaveLength(1)
    const mono = monoLines(m).map((el) => textOf(el.props.children).trim())
    expect(mono).toEqual([KEPT])
    expect(visibleText(m)).toContain(ROW_SENTENCE)
    expect(visibleErrorSurfaces(m)).toEqual([])
  })

  test('a repair that kept nothing shows no kept-files sentence and no folder', async () => {
    for (const preserved_location of [null, undefined]) {
      const { m } = await openSync({ repair: () => ({ pending_operations_preserved: 2, warnings: [], preserved_location }) })
      await press(m, 'Repair…')
      await press(m, 'Repair')
      expect(keptNotes(m)).toHaveLength(0)
      expect(visibleText(m)).not.toContain(ROW_SENTENCE)
      expect(monoLines(m)).toHaveLength(0)
      expect(statuses(m).map((el) => textOf(el.props.children).trim())).toContain('2 changes waiting to upload were kept.')
    }
  })

  test('a repair that fails is ONE inline alert (spec section 7), no toast', async () => {
    const { m } = await openSync({ repair: () => { throw new Error('socket busy') } })
    await press(m, 'Repair…')
    await press(m, 'Repair')
    expect(find(m, (el) => el.props['data-error-surface'] === 'finder-repair')).toHaveLength(1)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    expect(m.toasts).toEqual([])
    expect(visibleText(m)).not.toContain('socket busy')
  })

  // 1882 r4 (lead ruling): after the Finder location was removed, "Nothing was changed that you need
  // to undo" is false. The two failures read differently; the raw detail is never shown.
  const alertText = (m: Mounted) => find(m, (el) => el.props['data-error-surface'] === 'finder-repair').map((el) => readable(expand(el)).join(' '))

  test('a repair that fails before the removal keeps the old copy, word for word', async () => {
    const { m } = await openSync({ repair: () => { throw new Error('socket busy') } })
    await press(m, 'Repair…')
    await press(m, 'Repair')
    const [text] = alertText(m)
    expect(text).toContain('Couldn’t repair Beebeeb in Finder')
    expect(text).toContain('Nothing was changed that you need to undo. Try again.')
    expect(text).not.toContain('was removed from Finder')
  })

  test('a repair that fails after the removal says Beebeeb was removed and how to add it back, in one alert', async () => {
    const { m } = await openSync({
      removesBeforeFailing: true,
      repair: () => { throw new Error(`${model.REPAIR_FAILED_AFTER_REMOVAL_CODE}: No space left on device`) },
    })
    await press(m, 'Repair…')
    await press(m, 'Repair')
    expect(find(m, (el) => el.props['data-error-surface'] === 'finder-repair')).toHaveLength(1)
    expect(visibleErrorSurfaces(m)).toHaveLength(1)
    const [text] = alertText(m)
    expect(text).toContain('Beebeeb was removed from Finder')
    expect(text).toContain('Repair couldn’t finish, so Beebeeb is no longer in Finder. Choose Add to Finder to add it back.')
    expect(visibleText(m)).not.toContain('Nothing was changed that you need to undo')
    expect(visibleText(m)).not.toContain('Couldn’t repair Beebeeb in Finder')
    expect(visibleText(m)).not.toContain('No space left')
    expect(m.toasts).toEqual([])
    expect(buttons(m)).toContain('Add to Finder') // the sentence is true: the button it names is there
  })

  test('a successful repair whose refreshed Finder state cannot be read stops claiming Added and offers Try again', async () => {
    let failRead = false
    const st = { finder: finder.installed }
    const m = open('SyncTab', {
      finder_location_state: () => {
        if (failRead) throw new Error('offline')
        return st.finder
      },
      reset_macos_integration: () => {
        st.finder = finder.missing // the repair really removed the integration
        return { removed_file_provider_domain: true, disabled_autostart: true, removed_socket: true, removed_cache_files: 0, skipped_cache_files: 0, pending_operations_preserved: 0, warnings: [] }
      },
      list_remote_tree: () => [folder('a', 'Photos', true)],
      open_login_items_and_extensions_settings: () => undefined,
    }, { props: { settings: ready() } })
    await m.flush()
    expect(visibleText(m)).toContain('Added')
    await press(m, 'Repair…')
    failRead = true
    await press(m, 'Repair')
    // The reset succeeded but the refresh failed: the row must not keep the pre-repair "Added"
    // (the integration is gone), and the failed refresh needs its own way back in.
    expect(visibleText(m)).not.toContain('Added')
    expect(visibleText(m)).toContain('Couldn’t check Finder.')
    expect(buttons(m)).toContain('Try again')
    failRead = false
    await press(m, 'Try again')
    expect(visibleText(m)).toContain('Add it to see your files in Finder like any other folder.')
    expect(buttons(m)).toContain('Add to Finder')
  })

  test('Keep on this Mac: the count of kept folders and a way to choose', async () => {
    const { m } = await openSync({ tree: [folder('a', 'Photos', true, [folder('a1', 'Trips', true)]), folder('b', 'Work', false)] })
    expect(visibleText(m)).toContain('Keep on this Mac')
    expect(visibleText(m)).toContain('Everything else downloads when you open it.')
    expect(visibleText(m)).toContain('2 folders')
    expect(buttons(m)).toContain('Choose folders…')
  })

  test('when the backend reports no pin state the row says so instead of "0 folders", and the sheet cannot open', async () => {
    const { m } = await openSync({ tree: [folder('a', 'Photos', undefined)] })
    expect(visibleText(m)).toContain('Not available in this version yet.')
    expect(visibleText(m)).not.toContain('No folders')
    expect(button(m, 'Choose folders…').props.disabled).toBe(true)
  })

  test('a vault that cannot be listed is an inline row state with Try again, not a toast', async () => {
    const { backend } = syncBackend()
    let fail = true
    const m = open('SyncTab', { ...backend, list_remote_tree: () => { if (fail) throw new Error('offline'); return [folder('a', 'Photos', true)] } }, { props: { settings: ready() } })
    await m.flush()
    expect(visibleText(m)).toContain('Couldn’t load your folders.')
    expect(m.toasts).toEqual([])
    fail = false
    await press(m, 'Try again')
    expect(visibleText(m)).toContain('1 folder')
  })

  test('the sheet lists every folder with a switch; flipping one sends the pin and updates the count', async () => {
    const pins: any[] = []
    const { m } = await openSync({ pin: (a) => { pins.push(a) }, tree: [folder('a', 'Photos', true, [folder('a1', 'Trips', false)]), folder('b', 'Work', false)] })
    await press(m, 'Choose folders…')
    expect(dialogs(m)).toHaveLength(1)
    const switches = find(m, (el) => el.props.role === 'switch')
    expect(switches.map((s) => s.props['aria-checked'])).toEqual([true, false, false])
    await switches[1].props.onClick()
    await m.flush()
    expect(pins).toEqual([{ itemId: 'a1', pinned: true }])
    expect(visibleText(m)).toContain('2 folders')
  })

  test('a folder that cannot be kept is ONE toast and the switch stays off', async () => {
    const { m } = await openSync({ pin: () => { throw new Error('Choose a sync folder first.') }, tree: [folder('a', 'Photos', false)] })
    await press(m, 'Choose folders…')
    await find(m, (el) => el.props.role === 'switch')[0].props.onClick()
    await m.flush()
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t keep that folder on this Mac' })
    expect(find(m, (el) => el.props.role === 'switch')[0].props['aria-checked']).toBe(false)
  })

  test('Speed: two selects with a plain "No limit", and a change hands over exactly one key', async () => {
    const patches: any[] = []
    const settings = { ...ready(), save: async (patch: any) => { patches.push(patch) } }
    const { m } = await openSync({}, settings)
    const selects = find(m, (el) => el.type === 'select')
    expect(selects.map((s) => s.props.value)).toEqual([0, 5000])
    expect(String(selects[0].props['aria-labelledby'])).toBe('ms-upload-label')
    expect(String(selects[1].props['aria-labelledby'])).toBe('ms-download-label')
    selects[1].props.onChange({ target: { value: '10000' } })
    selects[0].props.onChange({ target: { value: '2000' } })
    expect(patches).toEqual([{ download_kbps_limit: 10000 }, { upload_kbps_limit: 2000 }])
  })

  test('a config that failed to load replaces the Speed group with one inline alert', async () => {
    const settings = { state: { status: 'failed' }, save: async () => {}, reload: async () => {} }
    const { m } = await openSync({}, settings)
    expect(find(m, (el) => el.props['data-error-surface'] === 'settings-config')).toHaveLength(1)
    expect(find(m, (el) => el.type === 'select')).toHaveLength(0)
    expect(visibleText(m)).not.toContain('Speed')
  })
})

// ── About ───────────────────────────────────────────────────────────────────

describe('About tab', () => {
  function updateStore(initial: any = { kind: 'idle' }) {
    const store = { state: initial, checks: 0, getSnapshot: () => store.state, subscribe: () => () => {}, check: async () => { store.checks += 1 } }
    return store
  }
  const openAbout = async (backend: Record<string, (a: any) => unknown> = {}, store = updateStore()) => {
    const m = open('AboutTab', { app_version: () => '0.8.6', report_problem: () => ({ path: '/tmp/bundle.json', email_opened: true }), install_update: () => undefined, 'plugin:opener|open_url': () => undefined, ...backend }, { bindings: { desktopUpdateCheck: store } })
    await m.flush()
    return { m, store }
  }

  test('the support-bundle row uses the exact copy task 1685 pinned to the Rust export, not a paraphrase', async () => {
    const { m } = await openAbout()
    const hint = find(m, (el) => el.props.id === 'ms-bundle-hint')
    expect(hint).toHaveLength(1)
    expect(textOf(hint[0].props.children)).toBe(diagnosticsCopy.SUPPORT_BUNDLE_DETAIL)
    expect(textOf(hint[0].props.children)).toContain('Paths, file and folder names and sign-in tokens are removed')
    expect(textOf(hint[0].props.children)).not.toMatch(/plaintext names|without secrets/i)
  })

  test('shows the version and the three things the spec lists, and nothing else to click', async () => {
    const { m } = await openAbout()
    const text = visibleText(m)
    expect(text).toContain('Beebeeb for Mac')
    expect(text).toContain('Version 0.8.6')
    expect(text).toContain('Get help')
    expect(text).toContain('Export a support bundle')
    expect(buttons(m)).toEqual(['Check for updates', 'Export…'])
    expect(find(m, (el) => el.type === 'button' && /Get help/.test(String(el.props['aria-label'])))).toHaveLength(1)
  })

  test('Export… writes the bundle through report_problem and says where, once, as a success toast', async () => {
    const { m } = await openAbout()
    await press(m, 'Export…')
    expect(m.calls.filter((c) => c.name === 'report_problem')).toHaveLength(1)
    expect(m.calls.filter((c) => c.name === 'export_diagnostics')).toHaveLength(0)
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'success', title: diagnosticsCopy.SUPPORT_BUNDLE_SAVED_TITLE })
    expect(m.toasts[0].message).toContain('/tmp/bundle.json')
  })

  test('a bundle saved while the email draft failed to open is a warning, not a failed save', async () => {
    const { m } = await openAbout({ report_problem: () => ({ path: '/tmp/bundle.json', email_opened: false }) })
    await press(m, 'Export…')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0].variant).toBe('warning')
    expect(m.toasts[0].message).toContain('send the file to support@beebeeb.io yourself')
  })

  test('a bundle that cannot be written is one error toast', async () => {
    const { m } = await openAbout({ report_problem: () => { throw new Error('disk full') } })
    await press(m, 'Export…')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Couldn’t save the support bundle' })
  })

  test('Get help opens the support page', async () => {
    const { m } = await openAbout()
    await find(m, (el) => el.type === 'button' && /Get help/.test(String(el.props['aria-label'])))[0].props.onClick()
    await m.flush()
    expect(m.calls.find((c) => c.name === 'plugin:opener|open_url')?.args).toEqual({ url: model.HELP_URL })
  })

  test('Check for updates asks the shared check once; the answer is the row\'s own hint (no toast)', async () => {
    const { m, store } = await openAbout()
    await press(m, 'Check for updates')
    expect(store.checks).toBe(1)
    store.state = { kind: 'up_to_date', currentVersion: '0.8.6', channel: 'stable' }
    m.render()
    expect(visibleText(m)).toContain('Version 0.8.6 is the latest.')
    expect(m.toasts).toEqual([])
  })

  test('while a check runs the button is disabled; an available update turns it into Restart to update', async () => {
    const checking = await openAbout({}, updateStore({ kind: 'checking' }))
    expect(button(checking.m, 'Check for updates').props.disabled).toBe(true)
    const available = await openAbout({}, updateStore({ kind: 'update_available', currentVersion: '0.8.6', channel: 'stable', version: '0.8.7' }))
    expect(visibleText(available.m)).toContain('Version 0.8.7 is available.')
    expect(buttons(available.m)).toContain('Restart to update')
    expect(buttons(available.m)).not.toContain('Check for updates')
    await press(available.m, 'Restart to update')
    expect(available.m.calls.filter((c) => c.name === 'install_update')).toHaveLength(1)
  })

  test('an update that fails to install is one error toast', async () => {
    const { m } = await openAbout({ install_update: () => { throw new Error('signature mismatch') } }, updateStore({ kind: 'update_available', currentVersion: '0.8.6', channel: 'stable', version: '0.8.7' }))
    await press(m, 'Restart to update')
    expect(m.toasts).toHaveLength(1)
    expect(m.toasts[0]).toMatchObject({ variant: 'error', title: 'Update install failed' })
  })
})

// ── The window ──────────────────────────────────────────────────────────────

describe('MacSettings window', () => {
  const tabStub = (id: string) => () => createElement('div', { 'data-tab': id })
  const settingsStub = () => ({ state: { status: 'loading' }, save: async () => {}, reload: async () => {} })
  const openWindow = (props: any = { initialTab: 'general' }, bindings: Record<string, unknown> = {}) =>
    open('MacSettings', {}, { props, bindings: { GeneralTab: tabStub('general'), AccountTab: tabStub('account'), SyncTab: tabStub('sync'), AboutTab: tabStub('about'), useSettingsConfig: settingsStub, listen: () => Promise.resolve(() => {}), connectNativeUpdateMenu: () => () => {}, ...bindings } })

  const shown = (m: Mounted) => find(m, (el) => el.props['data-tab'] !== undefined).map((el) => el.props['data-tab'])
  const tabs = (m: Mounted) => find(m, (el) => el.props.role === 'tab')

  test('four tabs in a tablist, the first selected, and exactly one tab mounted at a time', () => {
    const m = openWindow()
    expect(tabs(m).map((t) => textOf(t.props.children).trim())).toEqual(['General', 'Account', 'Sync', 'About'])
    expect(tabs(m).map((t) => t.props['aria-selected'])).toEqual([true, false, false, false])
    expect(shown(m)).toEqual(['general'])
    expect(find(m, (el) => el.props.role === 'tablist')).toHaveLength(1)
  })

  test('the window drains the native update-check menu request on mount (slice 6)', async () => {
    // Slice 6 flips the Preferences/Settings menu items and "Check for
    // updates…" onto this window: a native menu event must make the WINDOW
    // consume the pending request (the controller then runs the check the
    // About row renders — that behavior is nativeUpdateSettings's). Wired
    // through the real helper with a scripted backend (the consume handler
    // must live in the BACKEND: function-valued props are component props,
    // not command handlers).
    let onMenu: () => void = () => {}
    const m = open('MacSettings', { consume_menu_update_check: () => false }, {
      props: { initialTab: 'general' },
      bindings: {
        GeneralTab: tabStub('general'), AccountTab: tabStub('account'), SyncTab: tabStub('sync'), AboutTab: tabStub('about'),
        useSettingsConfig: settingsStub,
        listen: (_event: string, callback: () => void) => { onMenu = callback; return Promise.resolve(() => {}) },
        connectNativeUpdateMenu: realConnectNativeUpdateMenu,
        desktopUpdateCheck,
      },
    })
    await m.flush()
    // The helper drains once as soon as it is listening (the cold-window case:
    // the request may have been stored before this webview subscribed).
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(1)
    onMenu()
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(2)
  })

  test('a consumed menu update-check lands the window on the About tab', async () => {
    // "Check for updates…" must show its progress and result. The row that
    // renders them lives in the About tab, so the window must switch there
    // exactly when the pending menu request is consumed — not on a plain
    // open (which consumes false and keeps the General tab).
    const m = open('MacSettings', { consume_menu_update_check: () => true }, {
      props: { initialTab: 'general' },
      bindings: {
        GeneralTab: tabStub('general'), AccountTab: tabStub('account'), SyncTab: tabStub('sync'), AboutTab: tabStub('about'),
        useSettingsConfig: settingsStub,
        listen: () => Promise.resolve(() => {}),
        connectNativeUpdateMenu: realConnectNativeUpdateMenu,
        desktopUpdateCheck: { check: async () => {} },
      },
    })
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(1)
    expect(tabs(m).map((t) => t.props['aria-selected'])).toEqual([false, false, false, true])
    expect(shown(m)).toEqual(['about'])
  })

  test('a plain open consumes nothing and stays on the General tab', async () => {
    const m = open('MacSettings', { consume_menu_update_check: () => false }, {
      props: { initialTab: 'general' },
      bindings: {
        GeneralTab: tabStub('general'), AccountTab: tabStub('account'), SyncTab: tabStub('sync'), AboutTab: tabStub('about'),
        useSettingsConfig: settingsStub,
        listen: () => Promise.resolve(() => {}),
        connectNativeUpdateMenu: realConnectNativeUpdateMenu,
        desktopUpdateCheck: { check: async () => {} },
      },
    })
    await settle(m)
    expect(m.calls.filter((c) => c.name === 'consume_menu_update_check')).toHaveLength(1)
    expect(tabs(m).map((t) => t.props['aria-selected'])).toEqual([true, false, false, false])
    expect(shown(m)).toEqual(['general'])
  })

  test('clicking a tab swaps the panel and moves selection and the one tab stop', () => {
    const m = openWindow()
    tabs(m)[2].props.onClick()
    m.render()
    expect(shown(m)).toEqual(['sync'])
    expect(tabs(m).map((t) => t.props['aria-selected'])).toEqual([false, false, true, false])
    expect(tabs(m).map((t) => t.props.tabIndex)).toEqual([-1, -1, 0, -1])
    const panel = find(m, (el) => el.props.role === 'tabpanel')[0]
    expect(panel.props['aria-labelledby']).toBe('ms-tab-sync')
    expect(tabs(m)[2].props.id).toBe('ms-tab-sync')
  })

  test('the arrow keys move between tabs and the window consumes only those keys', () => {
    const m = openWindow()
    const tablist = find(m, (el) => el.props.role === 'tablist')[0]
    let prevented = 0
    tablist.props.onKeyDown({ key: 'ArrowRight', preventDefault: () => { prevented += 1 } })
    m.render()
    expect(shown(m)).toEqual(['account'])
    tablist.props.onKeyDown({ key: 'Tab', preventDefault: () => { prevented += 1 } })
    m.render()
    expect(shown(m)).toEqual(['account'])
    expect(prevented).toBe(1)
  })

  test('the toolbar is the drag region, so the window can be moved by it', () => {
    const m = openWindow()
    expect(find(m, (el) => el.props['data-tauri-drag-region'] !== undefined)).toHaveLength(1)
    expect(find(m, (el) => el.props.role === 'tablist')[0].props['data-tauri-drag-region']).toBeDefined()
  })

  test('an initial tab from the caller wins; with none, the URL decides', () => {
    expect(shown(openWindow({ initialTab: 'about' }))).toEqual(['about'])
    expect(shown(openWindow({}, { settingsTabFromLocation: () => 'sync' }))).toEqual(['sync'])
  })

  test('the shell is the only layout root and has no sidebar', () => {
    const m = openWindow()
    const root = m.tree()
    expect(root.props.className).toBe('ms-shell')
    expect(find(m, (el) => /sidebar/i.test(String(el.props.className ?? '')))).toHaveLength(0)
  })
})

// ── Confirmation dialogs: danger + Enter (task 1683 follow-up, ruling 2026-10-02) ──
//
// The ruling: the Sign out confirm is the destructive red (`ms-btn--danger`) because stopping
// sync is not offered as reversible; Repair stays amber because it is reversible. Enter
// confirms only when the confirm button itself is focused — the handler lives ON the button,
// so on open (focus is the close button) a stray Return resolves to Cancel natively and never
// reaches the confirm. Choose folders: Enter stays a no-op.

describe('Confirmation dialogs: danger and Enter', () => {
  const readySettings = { state: { status: 'ready', config }, save: async () => {}, reload: async () => {} }
  const accountBackend = () => ({
    popover_snapshot: () => snapshot(),
    account_subscription: () => subscription,
    lock_vault: () => undefined,
    unlock_vault: () => undefined,
    clear_session: () => undefined,
  })
  const repairBackend = (over: Record<string, (a: any) => unknown> = {}) => ({
    finder_location_state: () => finder.installed,
    install_finder_location: () => finder.installed,
    reset_macos_integration: () => ({ removed_file_provider_domain: true, disabled_autostart: true, removed_socket: true, removed_cache_files: 0, skipped_cache_files: 0, pending_operations_preserved: 0, warnings: [] }),
    list_remote_tree: () => [],
    open_login_items_and_extensions_settings: () => undefined,
    ...over,
  })
  const openSyncRepair = async (over: Record<string, (a: any) => unknown> = {}) => {
    const m = open('SyncTab', repairBackend(over), { props: { settings: readySettings } })
    await m.flush()
    return m
  }
  /** Dispatch Enter the way the harness does for other keys; report whether default was prevented. */
  const keyEnter = (el: TreeNode): number => {
    let prevented = 0
    el.props.onKeyDown({ key: 'Enter', preventDefault: () => { prevented += 1 } })
    return prevented
  }

  test('the Sign out confirm renders as destructive red, Cancel stays neutral, and the order stays Cancel → Sign out', async () => {
    const m = open('AccountTab', accountBackend())
    await settle(m)
    await press(m, 'Sign out…')
    expect(dialogs(m)).toHaveLength(1)
    const confirm = button(m, 'Sign out')
    expect(confirm.props.className).toContain('ms-btn--danger')
    expect(confirm.props.className).not.toContain('ms-btn--primary')
    expect(button(m, 'Cancel').props.className).not.toContain('ms-btn--danger')
    const flat = m.elements().map((el) => (el.type === 'button' ? textOf(el.props.children).trim() : null))
    expect(flat.indexOf('Cancel')).toBeGreaterThanOrEqual(0)
    expect(flat.indexOf('Cancel')).toBeLessThan(flat.indexOf('Sign out'))
  })

  test('the Repair confirm keeps the amber primary, never the danger red', async () => {
    const m = await openSyncRepair()
    await press(m, 'Repair…')
    const confirm = button(m, 'Repair')
    expect(confirm.props.className).toContain('ms-btn--primary')
    expect(confirm.props.className).not.toContain('ms-btn--danger')
  })

  test('on open a stray Return resolves to nothing: nothing is sent, and the only Enter handler in the sheet is the confirm button', async () => {
    const m = open('AccountTab', accountBackend())
    await settle(m)
    await press(m, 'Sign out…')
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(0)
    const handlers = find(m, (el) => typeof el.props.onKeyDown === 'function')
    expect(handlers).toHaveLength(1)
    expect(textOf(handlers[0].props.children).trim()).toBe('Sign out')
  })

  test('Enter on the focused confirm button confirms exactly once, and preventDefault keeps the browser click from confirming twice', async () => {
    const m = open('AccountTab', accountBackend())
    await settle(m)
    await press(m, 'Sign out…')
    expect(keyEnter(button(m, 'Sign out'))).toBe(1)
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(1)
  })

  test('Enter confirms Repair the same way, once', async () => {
    const m = await openSyncRepair()
    await press(m, 'Repair…')
    expect(keyEnter(button(m, 'Repair'))).toBe(1)
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'reset_macos_integration')).toHaveLength(1)
  })

  test('a key that is not Enter does not confirm through the handler', async () => {
    const m = open('AccountTab', accountBackend())
    await settle(m)
    await press(m, 'Sign out…')
    button(m, 'Sign out').props.onKeyDown({ key: ' ', preventDefault: () => {} })
    button(m, 'Sign out').props.onKeyDown({ key: 'Escape', preventDefault: () => {} })
    await m.flush()
    expect(m.calls.filter((c) => c.name === 'clear_session')).toHaveLength(0)
  })

  test('Choose folders: Enter is a no-op — no Enter handler anywhere on the sheet, nothing sent', async () => {
    const m = await openSyncRepair({ list_remote_tree: () => [folder('a', 'Photos', true), folder('b', 'Work', false)] })
    await press(m, 'Choose folders…')
    expect(dialogs(m)).toHaveLength(1)
    expect(find(m, (el) => typeof el.props.onKeyDown === 'function')).toHaveLength(0)
    for (const sw of find(m, (el) => el.props.role === 'switch')) expect(sw.props.onKeyDown).toBeUndefined()
    expect(m.calls.filter((c) => c.name === 'set_recursive_pin')).toHaveLength(0)
  })
})
